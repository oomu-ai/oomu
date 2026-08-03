#!/usr/bin/env python3
"""Local public-web search MCP server for OOMU.

The server intentionally uses only Python stdlib primitives. Search requests run
through a no-cookie, no-proxy opener against an allowlisted search HTML endpoint,
and HOME/cache/tmp are supplied by the Rust bootstrap as a dedicated app-data
profile rather than the user's normal browser profile.
"""

import html
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from html.parser import HTMLParser


PROTOCOL_VERSION = "2025-06-18"
SERVER_NAME = "local_search"
SERVER_VERSION = "1.0.0"
DEFAULT_PROFILE_ROOT = os.path.join(
    os.path.expanduser("~/Library/Application Support"),
    "ai.eldris.oomu.gpd",
    "mcp_search_profile",
)
PROFILE_ROOT = os.path.abspath(
    os.environ.get("OOMU_MCP_SEARCH_PROFILE_DIR", DEFAULT_PROFILE_ROOT)
)
SEARCH_ENDPOINT = "https://lite.duckduckgo.com/lite/"
ALLOWED_SEARCH_HOSTS = {"duckduckgo.com", "html.duckduckgo.com", "lite.duckduckgo.com"}
DEFAULT_TIMEOUT_SECONDS = 12
MAX_QUERY_CHARS = 500
MAX_RESULTS = 10
MAX_RESPONSE_BYTES = 2_000_000


class ToolInputError(ValueError):
    pass


class SearchFetchError(RuntimeError):
    pass


def ensure_profile_root():
    for relative in ("", "cache", "config", "tmp"):
        os.makedirs(os.path.join(PROFILE_ROOT, relative), exist_ok=True)


def text_result(text, structured=None):
    result = {
        "content": [{"type": "text", "text": text}],
        "isError": False,
    }
    if structured is not None:
        result["structuredContent"] = structured
    return result


def error_result(message):
    return {
        "content": [{"type": "text", "text": str(message)}],
        "isError": True,
    }


def text_arg(arguments, name, default=None, required=False, max_chars=MAX_QUERY_CHARS):
    value = arguments.get(name, default)
    if value is None:
        if required:
            raise ToolInputError(name + " is required.")
        return ""
    if isinstance(value, (dict, list)):
        raise ToolInputError(name + " must be text.")
    text = str(value).strip()
    if required and not text:
        raise ToolInputError(name + " is required.")
    if len(text) > max_chars:
        raise ToolInputError(name + " is too long.")
    return text


def int_arg(arguments, name, default, minimum, maximum):
    raw_value = arguments.get(name, default)
    try:
        value = int(raw_value)
    except (TypeError, ValueError) as exc:
        raise ToolInputError(name + " must be an integer.") from exc
    if value < minimum or value > maximum:
        raise ToolInputError(
            name + " must be between " + str(minimum) + " and " + str(maximum) + "."
        )
    return value


def clean_text(value):
    return " ".join(html.unescape(value).split())


def no_cookie_opener():
    return urllib.request.build_opener(
        urllib.request.ProxyHandler({}),
        urllib.request.HTTPHandler(),
        urllib.request.HTTPSHandler(),
    )


def decode_response_body(response, data):
    content_type = response.headers.get("Content-Type", "")
    charset = "utf-8"
    for part in content_type.split(";"):
        part = part.strip()
        if part.lower().startswith("charset="):
            charset = part.split("=", 1)[1].strip() or charset
    try:
        return data.decode(charset, errors="replace")
    except LookupError:
        return data.decode("utf-8", errors="replace")


def fetch_search_html(url):
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "https" or parsed.hostname not in ALLOWED_SEARCH_HOSTS:
        raise SearchFetchError("Search endpoint is outside the local_search allowlist.")

    request = urllib.request.Request(
        url,
        headers={
            "Accept": "text/html,application/xhtml+xml",
            "Accept-Language": "en-US,en;q=0.8",
            "Cache-Control": "no-store",
            "DNT": "1",
            "User-Agent": (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
                "AppleWebKit/605.1.15 (KHTML, like Gecko) "
                "Version/17.0 Safari/605.1.15"
            ),
        },
        method="GET",
    )
    try:
        with no_cookie_opener().open(request, timeout=DEFAULT_TIMEOUT_SECONDS) as response:
            data = response.read(MAX_RESPONSE_BYTES + 1)
            if len(data) > MAX_RESPONSE_BYTES:
                raise SearchFetchError("Search response exceeded the local size limit.")
            return decode_response_body(response, data)
    except urllib.error.URLError as exc:
        reason = getattr(exc, "reason", exc)
        raise SearchFetchError("Search request failed: " + str(reason)) from exc


def normalize_result_url(raw_href):
    href = html.unescape(str(raw_href or "")).strip()
    if not href:
        return ""
    if href.startswith("//"):
        href = "https:" + href
    elif href.startswith("/"):
        href = "https://duckduckgo.com" + href

    parsed = urllib.parse.urlparse(href)
    is_duckduckgo_redirect = (
        parsed.hostname in ALLOWED_SEARCH_HOSTS and parsed.path.startswith("/l/")
    )
    if is_duckduckgo_redirect:
        target = urllib.parse.parse_qs(parsed.query).get("uddg", [""])[0]
        return normalize_result_url(target)

    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        return ""
    return urllib.parse.urlunparse(
        (parsed.scheme, parsed.netloc, parsed.path, parsed.params, parsed.query, "")
    )


class DuckDuckGoHtmlParser(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.results = []
        self.capture_kind = None
        self.capture_href = ""
        self.capture_depth = 0
        self.capture_parts = []

    def handle_starttag(self, tag, attrs):
        if self.capture_kind is not None:
            self.capture_depth += 1
            return

        attributes = {key.lower(): value or "" for key, value in attrs}
        classes = set(attributes.get("class", "").split())
        if tag == "a" and (
            "result__a" in classes or "result-link" in classes
        ):
            self._start_capture("title", attributes.get("href", ""))
        elif "result__snippet" in classes or "result-snippet" in classes:
            self._start_capture("snippet", "")

    def handle_data(self, data):
        if self.capture_kind is not None:
            self.capture_parts.append(data)

    def handle_endtag(self, _tag):
        if self.capture_kind is None:
            return
        self.capture_depth -= 1
        if self.capture_depth <= 0:
            self._finish_capture()

    def _start_capture(self, kind, href):
        self.capture_kind = kind
        self.capture_href = href
        self.capture_depth = 1
        self.capture_parts = []

    def _finish_capture(self):
        kind = self.capture_kind
        href = self.capture_href
        text = clean_text("".join(self.capture_parts))
        self.capture_kind = None
        self.capture_href = ""
        self.capture_depth = 0
        self.capture_parts = []

        if kind == "title":
            url = normalize_result_url(href)
            if text and url:
                self.results.append({"title": text, "url": url, "snippet": ""})
        elif kind == "snippet" and text:
            for result in reversed(self.results):
                if not result.get("snippet"):
                    result["snippet"] = text
                    break


def parse_search_results(document):
    parser = DuckDuckGoHtmlParser()
    parser.feed(document)
    unique = []
    seen_urls = set()
    for result in parser.results:
        url = result.get("url", "")
        if not url or url in seen_urls:
            continue
        seen_urls.add(url)
        unique.append(result)
    return unique


def build_search_url(query):
    return SEARCH_ENDPOINT + "?" + urllib.parse.urlencode({"q": query})


def render_results_text(query, results):
    if not results:
        return (
            "Local web search returned no parseable results for "
            + json.dumps(query)
            + "."
        )
    lines = ["Local web search results for " + json.dumps(query) + ":"]
    for index, result in enumerate(results, start=1):
        lines.append(str(index) + ". " + result["title"])
        lines.append("   " + result["url"])
        if result.get("snippet"):
            lines.append("   " + result["snippet"])
    return "\n".join(lines)


def search_web(arguments):
    query = text_arg(arguments, "query", required=True)
    max_results = int_arg(arguments, "max_results", 5, 1, MAX_RESULTS)
    started_at = time.monotonic()
    document = fetch_search_html(build_search_url(query))
    results = parse_search_results(document)[:max_results]
    elapsed_ms = round((time.monotonic() - started_at) * 1000)
    structured = {
        "query": query,
        "engine": "duckduckgo_html",
        "resultCount": len(results),
        "results": results,
        "retrievalElapsedMs": elapsed_ms,
        "security": {
            "cookiesEnabled": False,
            "proxyEnvironmentEnabled": False,
            "endpointAllowlist": sorted(ALLOWED_SEARCH_HOSTS),
            "dedicatedProfile": True,
        },
    }
    return text_result(render_results_text(query, results), structured)


def network_annotations():
    return {
        "openWorldHint": True,
        "readOnlyHint": True,
        "destructiveHint": False,
    }


def tool_list():
    return [
        {
            "name": "search_web",
            "description": (
                "Search the public web through an isolated local headless browser "
                "utility with no cookies, no inherited browser profile, and no "
                "access to local application databases."
            ),
            "outputSchema": {
                "type": "object",
                "x-oomu-result-contract": {
                    "kind": "collection",
                    "path": "/structuredContent/results",
                    "emptyIsSuccess": True,
                },
                "properties": {
                    "structuredContent": {
                        "type": "object",
                        "properties": {
                            "results": {"type": "array", "items": {}}
                        },
                        "required": ["results"],
                        "additionalProperties": True,
                    }
                },
                "required": ["structuredContent"],
                "additionalProperties": True,
            },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Public web search query.",
                        "maxLength": MAX_QUERY_CHARS,
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of search results to return.",
                        "default": 5,
                        "minimum": 1,
                        "maximum": MAX_RESULTS,
                    },
                },
                "required": ["query"],
                "additionalProperties": False,
            },
            "annotations": network_annotations(),
            "_meta": {
                "oomu.requiresApproval": True,
                "oomu.boundary": "local_search_network",
                "oomu.isolation": "strict_env_no_cookie_no_proxy_profile",
            },
        }
    ]


def call_tool(params):
    name = params.get("name")
    arguments = params.get("arguments") or {}
    if not isinstance(arguments, dict):
        return error_result("Tool arguments must be an object.")

    try:
        if name == "search_web":
            return search_web(arguments)
        return error_result("Unknown tool: " + str(name))
    except Exception as exc:
        return error_result(str(exc))


def handle_request(message):
    method = message.get("method")
    params = message.get("params") or {}

    if method == "initialize":
        return {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
            "instructions": (
                "Search public web indexes only. The server uses a dedicated "
                "profile, no cookie jar, no proxy environment, and an allowlisted "
                "HTTPS search endpoint."
            ),
        }
    if method == "notifications/initialized":
        return None
    if method == "tools/list":
        return {"tools": tool_list()}
    if method == "tools/call":
        return call_tool(params)
    raise ValueError("Unsupported MCP method: " + str(method))


def send_response(identifier, result=None, error=None):
    response = {"jsonrpc": "2.0", "id": identifier}
    if error is not None:
        response["error"] = {"code": -32000, "message": str(error)}
    else:
        response["result"] = result
    print(json.dumps(response, separators=(",", ":")), flush=True)


def main():
    ensure_profile_root()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        identifier = None
        try:
            message = json.loads(line)
            identifier = message.get("id")
            result = handle_request(message)
            if identifier is not None and result is not None:
                send_response(identifier, result=result)
        except Exception as exc:
            if identifier is not None:
                send_response(identifier, error=exc)


if __name__ == "__main__":
    main()
