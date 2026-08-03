#!/usr/bin/env python3
import datetime
import hashlib
import json
import os
import secrets
import subprocess
import sys
import time


PROTOCOL_VERSION = "2025-06-18"
SERVER_NAME = "macos_applescript"
SERVER_VERSION = "1.0.0"
OSASCRIPT_PATH = os.environ.get("OOMU_OSASCRIPT_PATH", "osascript")
CALENDAR_APP_REFERENCE = os.environ.get("OOMU_CALENDAR_APP_PATH", "/System/Applications/Calendar.app")
DEFAULT_TIMEOUT_SECONDS = 20
MAIL_READ_TIMEOUT_SECONDS = 20
UI_AUTOMATION_TIMEOUT_SECONDS = 20
WATCHDOG_KILL_GRACE_SECONDS = 0.2
MAIL_DRAFT_SETTLE_ATTEMPTS = 8
MAIL_DRAFT_SETTLE_INTERVAL_SECONDS = 0.5
MAIL_SEND_SETTLE_ATTEMPTS = 20
MAIL_SEND_SETTLE_INTERVAL_SECONDS = 1
MAIL_CLEANUP_SETTLE_SECONDS = 1
MAX_TEXT_CHARS = 20000
MAX_UI_TEXT_ITEMS = 120

FIELD_SEPARATOR = "\x1f"
RECORD_SEPARATOR = "\x1e"

APPLE_APP_ALIASES = {
    "app store": "App Store",
    "books": "Books",
    "calendar": "Calendar",
    "contacts": "Contacts",
    "facetime": "FaceTime",
    "find my": "Find My",
    "freeform": "Freeform",
    "home": "Home",
    "keychain access": "Keychain Access",
    "mail": "Mail",
    "maps": "Maps",
    "messages": "Messages",
    "music": "Music",
    "news": "News",
    "notes": "Notes",
    "photos": "Photos",
    "podcasts": "Podcasts",
    "reminders": "Reminders",
    "safari": "Safari",
    "shortcuts": "Shortcuts",
    "stocks": "Stocks",
    "system settings": "System Settings",
    "tv": "TV",
    "weather": "Weather",
}

PERMISSION_BLOCKED_OR_TIMED_OUT_PAYLOAD = {
    "status": "permission_blocked_or_timed_out",
    "suggested_action": "manual_upload",
    "message": "OOMU is blocked by macOS security permissions. Prompt the user to manually attach or drag-and-drop the files (such as local_mail.json) into the chat.",
}


class ToolInputError(ValueError):
    pass


def applescript_string(value):
    text = "" if value is None else str(value)
    normalized = text.replace("\r\n", "\n").replace("\r", "\n")
    parts = []
    current = []
    for character in normalized:
        if character == '"':
            if current:
                parts.append('"' + "".join(current) + '"')
                current = []
            parts.append("quote")
        elif character == "\n":
            if current:
                parts.append('"' + "".join(current) + '"')
                current = []
            parts.append("linefeed")
        else:
            current.append(character)
    if current:
        parts.append('"' + "".join(current) + '"')
    return " & ".join(parts) if parts else '""'


def applescript_error_number(message):
    text = "" if message is None else str(message).strip()
    if not text.endswith(")"):
        return None
    start = text.rfind("(")
    if start < 0:
        return None
    try:
        return int(text[start + 1 : -1])
    except ValueError:
        return None


def applescript_error_payload(error_type, message):
    payload = {
        "status": "error",
        "error_type": error_type,
        "message": message,
    }
    error_number = applescript_error_number(message)
    if error_number is not None:
        payload["error_number"] = error_number
    return json.dumps(payload, separators=(",", ":"))


def parse_applescript_error(output):
    if not isinstance(output, str):
        return None
    output = output.strip()
    if not output.startswith("{"):
        return None
    try:
        payload = json.loads(output)
    except json.JSONDecodeError:
        return None
    if (
        isinstance(payload, dict)
        and payload.get("status") == "error"
        and isinstance(payload.get("error_type"), str)
    ):
        return payload
    return None


def applescript_tool_error_result(output):
    error = parse_applescript_error(output)
    if not error:
        return None
    return error_result(error.get("message") or "AppleScript execution failed.")


def degraded_collection_result(collection_name, metadata, error):
    message = error.get("message") or "AppleScript execution failed."
    empty_collection = []
    structured = dict(metadata)
    structured[collection_name] = empty_collection
    structured["warning"] = error.get("error_type") or "execution_failed"
    structured["error"] = message
    return error_result(
        json.dumps({"error": message, collection_name: empty_collection}, indent=2),
        structured,
    )


def run_applescript(script, timeout=DEFAULT_TIMEOUT_SECONDS):
    process = None
    try:
        process = subprocess.Popen(
            [OSASCRIPT_PATH, "-e", script],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        if process is not None:
            process.terminate()
            try:
                process.communicate(timeout=WATCHDOG_KILL_GRACE_SECONDS)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate()
        return applescript_error_payload(
            "timeout",
            "AppleScript execution timed out after " + str(timeout) + "s.",
        )
    except OSError as exc:
        return applescript_error_payload(
            "execution_failed",
            str(exc) or "AppleScript execution failed.",
        )

    if process.returncode != 0:
        message = (stderr or "").strip() or (stdout or "").strip()
        return applescript_error_payload(
            "execution_failed",
            message or "AppleScript execution failed.",
        )

    return stdout.strip()


def permission_blocked_or_timed_out_result():
    return error_result(
        json.dumps(PERMISSION_BLOCKED_OR_TIMED_OUT_PAYLOAD, indent=2),
        dict(PERMISSION_BLOCKED_OR_TIMED_OUT_PAYLOAD),
    )


def preflight_automation(script):
    output = run_applescript(script, timeout=DEFAULT_TIMEOUT_SECONDS)
    if parse_applescript_error(output):
        return False
    return output.strip() == "ok"


def preflight_calendar_automation():
    return preflight_automation(
        "\n".join(
            [
                "with timeout of " + str(DEFAULT_TIMEOUT_SECONDS) + " seconds",
                "tell application " + applescript_string(CALENDAR_APP_REFERENCE),
                "  set calendar_count to count of calendars",
                "end tell",
                "end timeout",
                "return \"ok\"",
            ]
        )
    )


def preflight_mail_automation():
    return preflight_automation(
        "\n".join(
            [
                "with timeout of " + str(DEFAULT_TIMEOUT_SECONDS) + " seconds",
                "tell application \"Mail\"",
                "  set account_count to count of accounts",
                "end tell",
                "end timeout",
                "return \"ok\"",
            ]
        )
    )


def mail_automation_preflight_failure():
    output = run_applescript(
        "\n".join(
            [
                "with timeout of " + str(DEFAULT_TIMEOUT_SECONDS) + " seconds",
                'tell application "Mail"',
                "  set account_count to count of accounts",
                "end tell",
                "end timeout",
                'return "ok"',
            ]
        ),
        timeout=DEFAULT_TIMEOUT_SECONDS,
    )
    error = parse_applescript_error(output)
    if error:
        return error
    if output.strip() != "ok":
        return {
            "status": "error",
            "error_type": "execution_failed",
            "message": "Mail automation preflight returned an unexpected result.",
        }
    return None


def mail_automation_error_result(error):
    error_type = error.get("error_type") if isinstance(error, dict) else None
    error_number = error.get("error_number") if isinstance(error, dict) else None
    if error_type == "timeout" or error_number == -1712:
        code = "mail_automation_timeout"
        message = "Mail did not respond in time. No draft was created."
    elif error_number == -1743:
        code = "mail_automation_permission_required"
        message = "OOMU needs permission to use Mail before it can create this draft."
    else:
        code = "mail_automation_unavailable"
        message = "Mail is not available for automation right now. No draft was created."
    return error_result(
        message,
        {
            "status": "error",
            "code": code,
            "failurePhase": "preflight",
            "saved": False,
            "verified": False,
            "cleanupState": "not_required",
            "cleanupVerified": True,
            "residualDraftPossible": False,
        },
    )


def preflight_reminders_automation():
    return preflight_automation(
        "\n".join(
            [
                "with timeout of " + str(DEFAULT_TIMEOUT_SECONDS) + " seconds",
                "tell application \"Reminders\"",
                "  set list_count to count of lists",
                "end tell",
                "end timeout",
                "return \"ok\"",
            ]
        )
    )


def text_arg(arguments, name, default=None, required=False, max_chars=MAX_TEXT_CHARS):
    value = arguments.get(name)
    if value is None and default is not None:
        value = default
    if value is None:
        if required:
            raise ToolInputError(name + " is required.")
        return ""
    if isinstance(value, (dict, list)):
        raise ToolInputError(name + " must be text.")
    text = str(value)
    if len(text) > max_chars:
        raise ToolInputError(name + " is too long.")
    if required and text.strip() == "":
        raise ToolInputError(name + " is required.")
    return text


def number_arg(arguments, name, default, minimum, maximum):
    raw_value = arguments.get(name, default)
    try:
        value = float(raw_value)
    except (TypeError, ValueError) as exc:
        raise ToolInputError(name + " must be a number.") from exc
    if value < minimum or value > maximum:
        raise ToolInputError(name + " must be between " + str(minimum) + " and " + str(maximum) + ".")
    return value


def boolean_arg(arguments, name, default=False):
    value = arguments.get(name, default)
    if not isinstance(value, bool):
        raise ToolInputError(name + " must be true or false.")
    return value


def parse_local_datetime(value, fallback=None):
    if value is None or str(value).strip() == "":
        return fallback

    text = str(value).strip()
    try:
        parsed = datetime.datetime.fromisoformat(text)
    except ValueError as exc:
        raise ToolInputError("Date values must use ISO 8601, for example 2026-06-16T09:00:00.") from exc

    if parsed.tzinfo is not None:
        parsed = parsed.astimezone().replace(tzinfo=None)
    return parsed


def applescript_date(value):
    hour = value.strftime("%I").lstrip("0") or "12"
    return (
        value.strftime("%A, %B ")
        + str(value.day)
        + value.strftime(", %Y ")
        + hour
        + value.strftime(":%M:%S %p")
    )


def text_result(text, structured=None):
    result = {
        "content": [{"type": "text", "text": text}],
        "isError": False,
    }
    if structured is not None:
        result["structuredContent"] = structured
    return result


def bool_result(success, structured=None):
    payload = {"success": bool(success)}
    if structured:
        payload.update(structured)
    return text_result("true" if success else "false", payload)


def error_result(message, structured=None):
    result = {
        "content": [{"type": "text", "text": message}],
        "isError": True,
    }
    if structured is not None:
        result["structuredContent"] = structured
    return result


def collection_output_schema(collection_name):
    return {
        "type": "object",
        "x-oomu-result-contract": {
            "kind": "collection",
            "path": "/structuredContent/" + collection_name,
            "emptyIsSuccess": True,
        },
        "properties": {
            "structuredContent": {
                "type": "object",
                "properties": {collection_name: {"type": "array", "items": {}}},
                "required": [collection_name],
                "additionalProperties": True,
            }
        },
        "required": ["structuredContent"],
        "additionalProperties": True,
    }


def tool_list():
    return [
        {
            "name": "trigger_system_notification",
            "description": "Fire a native macOS notification banner through AppleScript.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title_text": {
                        "type": "string",
                        "description": "Notification title.",
                        "default": "OOMU Core",
                    },
                    "subtitle_text": {
                        "type": "string",
                        "description": "Optional notification subtitle.",
                    },
                    "body_text": {
                        "type": "string",
                        "description": "Notification body text.",
                    },
                },
                "required": ["body_text"],
                "additionalProperties": False,
            },
        },
        {
            "name": "read_system_calendar",
            "description": "Read local macOS Calendar events in a bounded time window.",
            "outputSchema": collection_output_schema("events"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "calendar_name": {
                        "type": "string",
                        "description": "Optional Calendar name. Leave blank to use Calendar's default calendar.",
                    },
                    "hours_ahead": {
                        "type": "number",
                        "description": "Window size when end_date is omitted.",
                        "default": 8,
                        "minimum": 0.25,
                        "maximum": 720,
                    },
                    "start_date": {
                        "type": "string",
                        "description": "Optional local ISO 8601 start, for example 2026-06-16T09:00:00.",
                    },
                    "end_date": {
                        "type": "string",
                        "description": "Optional local ISO 8601 end, for example 2026-06-16T17:00:00.",
                    },
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "add_system_reminder",
            "description": "Create a native macOS Reminders task in a local reminder list.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "list_name": {
                        "type": "string",
                        "description": "Optional Reminder list name. Leave blank to read all reminder lists.",
                    },
                    "title": {
                        "type": "string",
                        "description": "Reminder title.",
                    },
                    "body": {
                        "type": "string",
                        "description": "Optional reminder notes.",
                    },
                    "due_date": {
                        "type": "string",
                        "description": "Optional local ISO 8601 due date.",
                    },
                },
                "required": ["title"],
                "additionalProperties": False,
            },
        },
        {
            "name": "draft_system_email",
            "description": "Open macOS Mail with a visible outgoing message draft.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": {
                        "type": "string",
                        "description": "Comma-separated recipient email addresses.",
                    },
                    "subject": {
                        "type": "string",
                        "description": "Draft subject line.",
                    },
                    "body": {
                        "type": "string",
                        "description": "Draft message body.",
                    },
                    "cc": {
                        "type": "string",
                        "description": "Optional comma-separated CC recipients.",
                    },
                    "bcc": {
                        "type": "string",
                        "description": "Optional comma-separated BCC recipients.",
                    },
                },
                "required": ["subject", "body"],
                "additionalProperties": False,
            },
        },
        {
            "name": "prepare_system_message",
            "description": "Open a visible Messages composer with one recipient and message, without sending it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recipient": {
                        "type": "string",
                        "description": "The phone number, email address, or contact name to place in the composer.",
                    },
                    "body": {
                        "type": "string",
                        "description": "The message to place in the composer. OOMU will not send it.",
                    },
                },
                "required": ["recipient", "body"],
                "additionalProperties": False,
            },
        },
        {
            "name": "capture_disposable_window",
            "description": "Capture only a temporary OOMU test window, verify its pixels, then discard the image.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": False,
            },
        },
        {
            "name": "preview_camera",
            "description": "Open a brief visible camera preview, then close it without saving any image or video.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": False,
            },
        },
        {
            "name": "send_system_email",
            "description": "Send one email through macOS Mail and verify exactly one matching message in Sent Mail.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": {"type": "string", "description": "Comma-separated To recipients."},
                    "subject": {"type": "string", "description": "Message subject line."},
                    "body": {"type": "string", "description": "Message body."},
                    "cc": {"type": "string", "description": "Optional comma-separated CC recipients."},
                    "bcc": {"type": "string", "description": "Optional comma-separated BCC recipients."},
                    "attachmentPath": {
                        "type": "string",
                        "description": "Optional exact local file to attach.",
                    },
                },
                "required": ["to", "subject", "body"],
                "additionalProperties": False,
            },
        },
        {
            "name": "create_system_note",
            "description": "Create a native macOS Notes note in a local Notes folder.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "folder_name": {
                        "type": "string",
                        "description": "Optional Notes folder name. Leave blank to use the default Notes folder when available.",
                    },
                    "title": {
                        "type": "string",
                        "description": "Note title.",
                    },
                    "body": {
                        "type": "string",
                        "description": "Note body.",
                    },
                },
                "required": ["title", "body"],
                "additionalProperties": False,
            },
        },
        {
            "name": "read_system_emails",
            "description": "Read recent emails from the inbox of the local macOS Mail application.",
            "outputSchema": collection_output_schema("emails"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_messages": {
                        "type": "number",
                        "description": "Maximum number of recent messages to retrieve.",
                        "default": 10,
                        "minimum": 1,
                        "maximum": 50,
                    },
                    "unread_only": {
                        "type": "boolean",
                        "description": "If true, retrieve only unread messages.",
                        "default": False,
                    },
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "read_system_notes",
            "description": "Read recent local macOS Notes metadata and bounded note body excerpts.",
            "outputSchema": collection_output_schema("notes"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_notes": {
                        "type": "number",
                        "description": "Maximum number of notes to retrieve.",
                        "default": 20,
                        "minimum": 1,
                        "maximum": 50,
                    },
                    "search_text": {
                        "type": "string",
                        "description": "Optional case-insensitive text filter for note title or body.",
                    },
                    "include_body": {
                        "type": "boolean",
                        "description": "If true, include bounded note body excerpts.",
                        "default": True,
                    },
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "read_system_contacts",
            "description": "Read local macOS Contacts entries with bounded contact fields.",
            "outputSchema": collection_output_schema("contacts"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_contacts": {
                        "type": "number",
                        "description": "Maximum number of contacts to retrieve.",
                        "default": 20,
                        "minimum": 1,
                        "maximum": 50,
                    },
                    "search_text": {
                        "type": "string",
                        "description": "Optional case-insensitive text filter for contact name, organization, email, or phone.",
                    },
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "read_system_music",
            "description": "Read bounded newest-added song metadata from the local Apple Music library without playback or mutation.",
            "outputSchema": collection_output_schema("songs"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_songs": {
                        "type": "number",
                        "description": "Maximum number of newest-added songs to retrieve.",
                        "default": 1,
                        "minimum": 1,
                        "maximum": 20,
                    },
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "read_system_photos",
            "description": "Read bounded photo metadata through OOMU's native PhotoKit boundary without exporting image data.",
            "outputSchema": collection_output_schema("photos"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "max_photos": {
                        "type": "number",
                        "description": "Maximum number of newest photo records to retrieve.",
                        "default": 1,
                        "minimum": 1,
                        "maximum": 20,
                    },
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "read_system_reminders",
            "description": "Read tasks from a local macOS Reminders list.",
            "outputSchema": collection_output_schema("reminders"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "list_name": {
                        "type": "string",
                        "description": "Reminder list name.",
                        "default": "Reminders",
                    },
                    "completed_only": {
                        "type": "boolean",
                        "description": "If true, retrieve completed tasks instead of uncompleted ones.",
                        "default": False,
                    },
                },
                "additionalProperties": False,
            },
        },
        {
            "name": "read_apple_app_ui",
            "description": "Read bounded visible UI text from an allowlisted Apple system app. This is a read-only fallback for apps without a structured AppleScript data API and may require macOS Accessibility permission.",
            "outputSchema": collection_output_schema("uiText"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "app_name": {
                        "type": "string",
                        "description": "Apple app name, for example Weather, Safari, Photos, Messages, or System Settings.",
                    },
                    "max_items": {
                        "type": "number",
                        "description": "Maximum visible UI text items to return.",
                        "default": 80,
                        "minimum": 1,
                        "maximum": 120,
                    },
                    "activate": {
                        "type": "boolean",
                        "description": "If true, bring the app to the foreground before reading visible UI text.",
                        "default": True,
                    },
                },
                "required": ["app_name"],
                "additionalProperties": False,
            },
        },
    ]


def trigger_system_notification(arguments):
    text_arg(arguments, "title_text", default="OOMU", max_chars=256)
    text_arg(arguments, "subtitle_text", default="", max_chars=256)
    text_arg(arguments, "body_text", required=True, max_chars=1024)
    return error_result(
        "Notification delivery requires OOMU's native app boundary.",
        {
            "code": "notification_native_boundary_required",
            "verified": False,
        },
    )


def prepare_system_message(arguments):
    recipient = text_arg(arguments, "recipient", required=True, max_chars=256).strip()
    body = text_arg(arguments, "body", required=True, max_chars=4000)
    if not recipient or "\n" in recipient or "\r" in recipient:
        raise ToolInputError("recipient must be one phone number, email address, or contact name.")
    if not body.strip():
        raise ToolInputError("body must contain a message.")

    # Messages does not expose a draft object through AppleScript. This bounded UI
    # adapter opens a new composer, fills it, and verifies both values while the
    # composer remains open. It never invokes the Send action.
    script = "\n".join(
        [
            "tell application \"Messages\" to activate",
            "delay 0.5",
            "with timeout of " + str(UI_AUTOMATION_TIMEOUT_SECONDS) + " seconds",
            "tell application \"System Events\"",
            "  tell process \"Messages\"",
            "    keystroke \"n\" using command down",
            "    delay 0.5",
            "    if not (exists window 1) then error \"Messages did not open a composer.\"",
            "    set recipient_field to focused UI element",
            "    try",
            "      set value of recipient_field to " + applescript_string(recipient),
            "    on error",
            "      keystroke " + applescript_string(recipient),
            "    end try",
            "    key code 36",
            "    delay 0.25",
            "    set body_field to focused UI element",
            "    try",
            "      set value of body_field to " + applescript_string(body),
            "    on error",
            "      keystroke " + applescript_string(body),
            "    end try",
            "    delay 0.25",
            "    set recipient_verified to false",
            "    set body_verified to false",
            "    set compose_open to false",
            "    set ui_items to entire contents of window 1",
            "    repeat with ui_item in ui_items",
            "      set item_value to \"\"",
            "      try",
            "        set item_value to value of ui_item as text",
            "      end try",
            "      if item_value contains " + applescript_string(recipient) + " then set recipient_verified to true",
            "      if item_value contains " + applescript_string(body) + " then set body_verified to true",
            "    end repeat",
            "    if recipient_verified and body_verified then set compose_open to true",
            "    return (recipient_verified as text) & \"\u001f\" & (body_verified as text) & \"\u001f\" & (compose_open as text)",
            "  end tell",
            "end tell",
            "end timeout",
        ]
    )
    output = run_applescript(script, timeout=UI_AUTOMATION_TIMEOUT_SECONDS)
    error = applescript_tool_error_result(output)
    if error:
        return error
    states = [value.strip().lower() == "true" for value in output.split(FIELD_SEPARATOR)]
    verified = len(states) == 3 and all(states)
    if not verified:
        return error_result(
            "Messages opened, but OOMU could not verify the prepared message.",
            {
                "code": "message_prepare_verification_failed",
                "verified": False,
                "sent": False,
            },
        )
    return text_result(
        "The message is ready in Messages. It has not been sent.",
        {
            "status": "prepared",
            "verified": True,
            "sent": False,
            "recipientSha256": hashlib.sha256(recipient.encode("utf-8")).hexdigest(),
            "bodySha256": hashlib.sha256(body.encode("utf-8")).hexdigest(),
        },
    )


def read_system_calendar(arguments):
    calendar_name = text_arg(arguments, "calendar_name", default="", max_chars=256)
    hours_ahead = number_arg(arguments, "hours_ahead", 8, 0.25, 720)
    now = datetime.datetime.now().replace(microsecond=0)
    start_date = parse_local_datetime(arguments.get("start_date"), now)
    end_date = parse_local_datetime(
        arguments.get("end_date"),
        start_date + datetime.timedelta(hours=hours_ahead),
    )
    if end_date < start_date:
        raise ToolInputError("end_date must be after start_date.")
    if not preflight_calendar_automation():
        return permission_blocked_or_timed_out_result()

    calendar_name = calendar_name.strip()
    script = "\n".join(
        [
            "set fsChar to character id 31",
            "set rsChar to character id 30",
            "set rowsOut to {}",
            "with timeout of " + str(DEFAULT_TIMEOUT_SECONDS) + " seconds",
            "tell application " + applescript_string(CALENDAR_APP_REFERENCE),
            "  set startWindow to date " + applescript_string(applescript_date(start_date)),
            "  set endWindow to date " + applescript_string(applescript_date(end_date)),
            calendar_target_calendars_line(calendar_name),
            "  repeat with targetCalendar in targetCalendars",
            "    set matchingEvents to every event of targetCalendar whose (start date is greater than or equal to startWindow) and (start date is less than or equal to endWindow)",
            "    repeat with eventItem in matchingEvents",
            "      set eventLocation to location of eventItem",
            "      if eventLocation is missing value then set eventLocation to \"\"",
            "      set eventSummary to summary of eventItem",
            "      if eventSummary is missing value then set eventSummary to \"\"",
            "      set eventStart to start date of eventItem as string",
            "      set eventEnd to end date of eventItem as string",
            "      set rowText to ((name of targetCalendar as string) & my fsChar & (eventSummary as string) & my fsChar & eventStart & my fsChar & eventEnd & my fsChar & (eventLocation as string))",
            "      set my rowsOut to my rowsOut & {rowText}",
            "    end repeat",
            "  end repeat",
            "end tell",
            "end timeout",
            "set AppleScript's text item delimiters to rsChar",
            "return rowsOut as text",
        ]
    )

    output = run_applescript(script)
    error = parse_applescript_error(output)
    if error:
        return degraded_collection_result(
            "events",
            {
                "calendarName": calendar_name,
                "startDate": start_date.isoformat(),
                "endDate": end_date.isoformat(),
            },
            error,
        )
    events = parse_calendar_rows(output)
    return text_result(
        json.dumps(events, indent=2),
        {
            "calendarName": calendar_name,
            "startDate": start_date.isoformat(),
            "endDate": end_date.isoformat(),
            "events": events,
        },
    )


def calendar_target_calendars_line(calendar_name):
    if calendar_name:
        return "  set targetCalendars to {calendar " + applescript_string(calendar_name) + "}"
    return "  set targetCalendars to calendars"


def parse_calendar_rows(output):
    if output.strip() == "":
        return []
    events = []
    for row in output.split(RECORD_SEPARATOR):
        if not row:
            continue
        columns = row.split(FIELD_SEPARATOR)
        while len(columns) < 5:
            columns.append("")
        events.append(
            {
                "calendar": columns[0],
                "name": columns[1],
                "startTime": columns[2],
                "endTime": columns[3],
                "location": columns[4],
            }
        )
    return events


def add_system_reminder(arguments):
    list_name = text_arg(arguments, "list_name", default="Reminders", max_chars=256)
    title = text_arg(arguments, "title", required=True, max_chars=512)
    body = text_arg(arguments, "body", default="", max_chars=MAX_TEXT_CHARS)
    due_date = parse_local_datetime(arguments.get("due_date"))

    properties = [
        "name:" + applescript_string(title),
        "body:" + applescript_string(body),
    ]
    if due_date is not None:
        properties.append("due date:date " + applescript_string(applescript_date(due_date)))
    if not preflight_reminders_automation():
        return permission_blocked_or_timed_out_result()

    script = "\n".join(
        [
            "tell application \"Reminders\"",
            "  tell list " + applescript_string(list_name),
            "    set new_reminder to make new reminder with properties {" + ", ".join(properties) + "}",
            "    return (id of new_reminder as string) & tab & (name of new_reminder as string)",
            "  end tell",
            "end tell",
        ]
    )

    output = run_applescript(script)
    error = applescript_tool_error_result(output)
    if error:
        return error
    reminder_id, _, reminder_title = output.partition("\t")
    return text_result(
        json.dumps({"id": reminder_id, "title": reminder_title or title}, indent=2),
        {
            "id": reminder_id,
            "title": reminder_title or title,
            "listName": list_name,
            "dueDate": due_date.isoformat() if due_date else None,
        },
    )


def draft_system_email(arguments):
    to_recipients = recipients_arg(arguments, "to", required=False)
    cc_recipients = recipients_arg(arguments, "cc", required=False)
    bcc_recipients = recipients_arg(arguments, "bcc", required=False)
    subject = text_arg(arguments, "subject", required=True, max_chars=998)
    body = text_arg(arguments, "body", required=True).replace("\r\n", "\n").replace("\r", "\n")
    reuse_existing_matching = boolean_arg(
        arguments, "reuse_existing_matching", default=False
    )
    verify_existing_only = boolean_arg(
        arguments, "verify_existing_only", default=False
    )
    preflight_failure = mail_automation_preflight_failure()
    if preflight_failure:
        return mail_automation_error_result(preflight_failure)

    if verify_existing_only:
        existing = inspect_exact_mail_draft(
            to_recipients, cc_recipients, bcc_recipients, subject, body
        )
        if existing.get("error"):
            return mail_inventory_failure(
                existing["error"],
                "mail_draft_review_required"
                if existing.get("residual")
                else "mail_draft_result_unverified",
                "postcondition",
                existing.get("residual", False),
            )
        if not existing.get("draftId"):
            return mail_inventory_failure(
                "Mail no longer contains the exact unsent draft.",
                "mail_draft_result_unverified",
                "postcondition",
                False,
            )
        return bool_result(
            True,
            {
                "draftId": existing["draftId"],
                "to": to_recipients,
                "cc": cc_recipients,
                "bcc": bcc_recipients,
                "subject": subject,
                "bodySha256": hashlib.sha256(body.encode("utf-8")).hexdigest(),
                "draftState": "outgoing_message",
                "sent": False,
                "saved": True,
                "verified": True,
                "exactMatchCount": 1,
                "uniquenessVerified": True,
                "reusedExisting": True,
                "postconditionOnly": True,
            },
        )

    if reuse_existing_matching:
        existing = find_exact_mail_draft(
            to_recipients, cc_recipients, bcc_recipients, subject, body
        )
        if existing.get("error"):
            return mail_inventory_failure(
                existing["error"],
                "mail_draft_review_required"
                if existing.get("residual")
                else "mail_draft_result_unverified",
                "existing_lookup",
                existing.get("residual", False),
            )
        if existing.get("draftId"):
            settled = wait_for_exact_mail_draft(
                to_recipients,
                cc_recipients,
                bcc_recipients,
                subject,
                body,
                expected_draft_id=existing["draftId"],
                initial_draft_id=existing["draftId"],
            )
            if settled.get("draftId") == existing["draftId"]:
                return mail_draft_success_result(
                    existing["draftId"],
                    to_recipients,
                    cc_recipients,
                    bcc_recipients,
                    subject,
                    body,
                    True,
                )
            return mail_inventory_failure(
                settled.get("error")
                or "Mail could not confirm that the existing draft finished saving.",
                "mail_draft_review_required",
                "postcondition",
                True,
            )

    existing_before_create = inspect_exact_mail_draft(
        to_recipients, cc_recipients, bcc_recipients, subject, body
    )
    if existing_before_create.get("error"):
        return mail_inventory_failure(
            existing_before_create["error"],
            "mail_draft_review_required"
            if existing_before_create.get("residual")
            else "mail_draft_result_unverified",
            "existing_lookup",
            existing_before_create.get("residual", False),
        )
    if existing_before_create.get("draftId"):
        return mail_inventory_failure(
            "Mail already contains this exact unsent draft. OOMU did not create another.",
            "mail_draft_review_required",
            "existing_lookup",
            True,
        )

    operation_token = "OOMU-" + secrets.token_hex(16)
    bootstrap_script = "\n".join(
        [
            'tell application "Mail"',
            "  set new_message to make new outgoing message with properties {subject:"
            + applescript_string(operation_token)
            + ', content:"", visible:false}',
            "  save new_message",
            '  return "ok"',
            "end tell",
        ]
    )
    bootstrap_output = run_applescript(
        bootstrap_script, timeout=UI_AUTOMATION_TIMEOUT_SECONDS
    ).strip()
    if applescript_tool_error_result(bootstrap_output) or bootstrap_output != "ok":
        cleanup_result = remove_mail_draft_by_subject_token(operation_token)
        return mail_draft_failure(
            "Mail could not prepare the draft for verified creation.",
            cleanup_result,
            "bootstrap",
        )

    prepared = wait_for_mail_draft_subject_token(operation_token)
    prepared_draft_id = prepared.get("draftId")
    if not prepared_draft_id:
        return mail_draft_failure(
            "Mail could not verify the prepared draft in the Drafts mailbox.",
            remove_mail_draft_by_subject_token(operation_token),
            "bootstrap",
            force_residual=prepared.get("residual") is True,
        )

    lines = [
        "tell application \"Mail\"",
        "  set target_subject to " + applescript_string(operation_token),
        "  set matching_messages to every outgoing message whose subject is target_subject",
        "  if (count of matching_messages) is not 1 then error \"draft identity unavailable\"",
        "  set new_message to item 1 of matching_messages",
        "  set subject of new_message to " + applescript_string(subject),
        "  set content of new_message to " + applescript_string(body),
        "  tell new_message",
    ]
    lines.extend(recipient_lines("to recipient", to_recipients))
    lines.extend(recipient_lines("cc recipient", cc_recipients))
    lines.extend(recipient_lines("bcc recipient", bcc_recipients))
    lines.extend(
        [
            "  end tell",
            "  set visible of new_message to true",
            "  save new_message",
            "  set saved_subject to subject of new_message as string",
            "  set saved_body to content of new_message as string",
            "  set saved_to to my recipient_addresses(to recipients of new_message)",
            "  set saved_cc to my recipient_addresses(cc recipients of new_message)",
            "  set saved_bcc to my recipient_addresses(bcc recipients of new_message)",
            "  set saved_class to class of new_message as string",
            "  activate",
            "  return saved_subject & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_body & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_to & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_cc & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_bcc & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_class",
            "end tell",
            "on recipient_addresses(recipients_list)",
            "  tell application \"Mail\"",
            "    set collected to {}",
            "    repeat with recipient_ref in recipients_list",
            "      set recipient_value to contents of recipient_ref",
            "      set end of collected to (get address of recipient_value) as string",
            "    end repeat",
            "    set AppleScript's text item delimiters to " + applescript_string(","),
            "    set joined to collected as string",
            "    set AppleScript's text item delimiters to \"\"",
            "    return joined",
            "  end tell",
            "end recipient_addresses",
        ]
    )

    output = run_applescript("\n".join(lines), timeout=UI_AUTOMATION_TIMEOUT_SECONDS)
    output_error = applescript_tool_error_result(output)
    fields = [] if output_error else output.split(FIELD_SEPARATOR)
    while len(fields) < 6:
        fields.append("")
    (
        saved_subject,
        saved_body,
        saved_to,
        saved_cc,
        saved_bcc,
        saved_class,
    ) = fields[:6]
    saved_fields_verified = output_error is None and (
        saved_subject == subject
        and mail_body_readback_matches(body, saved_body)
        and [item for item in saved_to.split(",") if item] == to_recipients
        and [item for item in saved_cc.split(",") if item] == cc_recipients
        and [item for item in saved_bcc.split(",") if item] == bcc_recipients
        and saved_class.strip().lower() == "outgoing message"
    )
    settled = wait_for_exact_mail_draft(
        to_recipients,
        cc_recipients,
        bcc_recipients,
        subject,
        body,
    )
    final_draft_id = settled.get("draftId")
    if final_draft_id:
        token_absence = wait_for_mail_draft_subject_token_absence(operation_token)
        if token_absence.get("absent"):
            return mail_draft_success_result(
                final_draft_id,
                to_recipients,
                cc_recipients,
                bcc_recipients,
                subject,
                body,
                False,
            )
        settled = token_absence
    if output_error:
        failure_message = "Mail could not verify the saved draft after creation."
        failure_phase = "populate_verify"
    elif not saved_fields_verified:
        failure_message = "Mail did not return a matching saved draft after creation."
        failure_phase = "populate_verify"
    else:
        failure_message = (
            "Mail could not verify exactly one matching unsent draft after creation."
        )
        failure_phase = "postcondition"
    return mail_draft_failure(
        failure_message,
        remove_mail_draft_by_id(
            prepared_draft_id,
            operation_token=operation_token,
            to_recipients=to_recipients,
            cc_recipients=cc_recipients,
            bcc_recipients=bcc_recipients,
            subject=subject,
            body=body,
        ),
        failure_phase,
        force_residual=settled.get("residual") is True,
    )


def send_system_email(arguments):
    to_recipients = recipients_arg(arguments, "to", required=True)
    cc_recipients = recipients_arg(arguments, "cc", required=False)
    bcc_recipients = recipients_arg(arguments, "bcc", required=False)
    subject = text_arg(arguments, "subject", required=True, max_chars=998)
    body = text_arg(arguments, "body", required=True).replace("\r\n", "\n").replace("\r", "\n")
    attachment_path = text_arg(
        arguments, "attachmentPath", required=False, max_chars=4096
    ).strip()
    attachment_name = ""
    attachment_sha256 = ""
    attachment_bytes = 0
    if attachment_path:
        if (
            not os.path.isabs(attachment_path)
            or os.path.islink(attachment_path)
            or not os.path.isfile(attachment_path)
        ):
            return mail_send_failure(
                "The verified email attachment is no longer available. No email was sent.",
                "mail_attachment_unavailable",
                "preflight",
                "none",
            )
        attachment_name = os.path.basename(attachment_path)
        attachment_sha256 = sha256_file(attachment_path)
        attachment_bytes = os.path.getsize(attachment_path)
    preflight_failure = mail_automation_preflight_failure()
    if preflight_failure:
        return mail_send_automation_error_result(preflight_failure)

    existing = inspect_exact_sent_email(
        to_recipients,
        cc_recipients,
        bcc_recipients,
        subject,
        body,
        attachment_name,
    )
    if existing.get("duplicates"):
        return mail_send_failure(
            "Mail contains multiple matching sent messages. OOMU did not send another.",
            "mail_send_duplicate_detected",
            "existing_lookup",
            "external_changes",
        )
    if existing.get("error"):
        return mail_send_failure(
            existing["error"],
            "mail_send_result_unverified",
            "existing_lookup",
            "unverified",
        )
    if existing.get("sentMessageId"):
        return mail_send_success_result(
            existing["sentMessageId"],
            to_recipients,
            cc_recipients,
            bcc_recipients,
            subject,
            body,
            attachment_name,
            attachment_sha256,
            attachment_bytes,
            True,
        )

    lines = [
        'tell application "Mail"',
        "  set new_message to make new outgoing message with properties {subject:"
        + applescript_string(subject)
        + ", content:"
        + applescript_string(body)
        + ", visible:false}",
        "  tell new_message",
    ]
    lines.extend(recipient_lines("to recipient", to_recipients))
    lines.extend(recipient_lines("cc recipient", cc_recipients))
    lines.extend(recipient_lines("bcc recipient", bcc_recipients))
    if attachment_path:
        lines.append(
            "    make new attachment with properties {file name:POSIX file "
            + applescript_string(attachment_path)
            + "} at after the last paragraph"
        )
    lines.extend(["  end tell", "  send new_message", '  return "ok"', "end tell"])
    send_output = run_applescript(
        "\n".join(lines), timeout=UI_AUTOMATION_TIMEOUT_SECONDS
    ).strip()
    send_error = applescript_tool_error_result(send_output)
    settled = wait_for_exact_sent_email(
        to_recipients,
        cc_recipients,
        bcc_recipients,
        subject,
        body,
        attachment_name,
    )
    if settled.get("sentMessageId"):
        return mail_send_success_result(
            settled["sentMessageId"],
            to_recipients,
            cc_recipients,
            bcc_recipients,
            subject,
            body,
            attachment_name,
            attachment_sha256,
            attachment_bytes,
            False,
        )
    if settled.get("duplicates"):
        return mail_send_failure(
            "Mail contains multiple matching sent messages.",
            "mail_send_duplicate_detected",
            "postcondition",
            "external_changes",
        )
    message = (
        "Mail accepted the send request but OOMU could not verify the sent message. Review Sent Mail before retrying."
        if send_error is None and send_output == "ok"
        else "Mail did not return a verifiable send result. Review Sent Mail before retrying."
    )
    return mail_send_failure(
        message,
        "mail_send_result_unverified",
        "postcondition",
        "unverified",
    )


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as input_file:
        while True:
            chunk = input_file.read(64 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def inspect_exact_sent_email(
    to_recipients,
    cc_recipients,
    bcc_recipients,
    subject,
    body,
    attachment_name="",
):
    duplicate_sentinel = "OOMU_MULTIPLE_MATCHING_SENT_MESSAGES"
    lines = [
        'tell application "Mail"',
        "  set expected_subject to " + applescript_string(subject),
        mail_body_variant_declaration(body),
        "  set expected_to to " + applescript_string(",".join(to_recipients)),
        "  set expected_cc to " + applescript_string(",".join(cc_recipients)),
        "  set expected_bcc to " + applescript_string(",".join(bcc_recipients)),
        "  set expected_attachment to " + applescript_string(attachment_name),
    ]
    lines.extend(
        mail_exact_inventory_lines(
            "every message of sent mailbox whose subject is expected_subject", "exact_sent"
        )
    )
    lines.extend(
        [
            "  set attachment_matched_sent to {}",
            "  repeat with candidate_message in exact_sent",
            "    if my attachment_names(candidate_message) is expected_attachment then set end of attachment_matched_sent to candidate_message",
            "  end repeat",
            "  set exact_sent to attachment_matched_sent",
            "  if (count of exact_sent) is greater than 1 then return "
            + applescript_string(duplicate_sentinel),
            '  if (count of exact_sent) is 0 then return ""',
            "  return id of item 1 of exact_sent as string",
            "end tell",
            "on recipient_addresses(recipients_list)",
            '  tell application "Mail"',
            "    set collected to {}",
            "    repeat with recipient_ref in recipients_list",
            "      set recipient_value to contents of recipient_ref",
            "      set end of collected to (get address of recipient_value) as string",
            "    end repeat",
            "    set AppleScript's text item delimiters to " + applescript_string(","),
            "    set joined to collected as string",
            '    set AppleScript\'s text item delimiters to ""',
            "    return joined",
            "  end tell",
            "end recipient_addresses",
            "on attachment_names(message_ref)",
            '  tell application "Mail"',
            "    set collected to {}",
            "    repeat with attachment_ref in (mail attachments of message_ref)",
            "      set end of collected to (name of attachment_ref as string)",
            "    end repeat",
            "    set AppleScript's text item delimiters to " + applescript_string(","),
            "    set joined to collected as string",
            '    set AppleScript\'s text item delimiters to ""',
            "    return joined",
            "  end tell",
            "end attachment_names",
        ]
    )
    output = run_applescript(
        "\n".join(lines), timeout=UI_AUTOMATION_TIMEOUT_SECONDS
    ).strip()
    if applescript_tool_error_result(output):
        return {"error": "Mail could not inspect Sent Mail safely."}
    if output == duplicate_sentinel:
        return {"duplicates": True}
    return {"sentMessageId": output} if output else {}


def wait_for_exact_sent_email(
    to_recipients,
    cc_recipients,
    bcc_recipients,
    subject,
    body,
    attachment_name="",
):
    last_message_id = None
    consecutive_matches = 0
    last_error = None
    for attempt in range(MAIL_SEND_SETTLE_ATTEMPTS):
        if attempt > 0:
            time.sleep(MAIL_SEND_SETTLE_INTERVAL_SECONDS)
        inspected = inspect_exact_sent_email(
            to_recipients,
            cc_recipients,
            bcc_recipients,
            subject,
            body,
            attachment_name,
        )
        if inspected.get("duplicates"):
            return inspected
        if inspected.get("error"):
            last_error = inspected
            last_message_id = None
            consecutive_matches = 0
            continue
        sent_message_id = inspected.get("sentMessageId")
        if sent_message_id:
            if sent_message_id == last_message_id:
                consecutive_matches += 1
            else:
                last_message_id = sent_message_id
                consecutive_matches = 1
            if consecutive_matches >= 2:
                return {"sentMessageId": sent_message_id, "settled": True}
        else:
            last_message_id = None
            consecutive_matches = 0
    return last_error or {}


def mail_send_success_result(
    message_id,
    to_recipients,
    cc_recipients,
    bcc_recipients,
    subject,
    body,
    attachment_name,
    attachment_sha256,
    attachment_bytes,
    reused_existing,
):
    return bool_result(
        True,
        {
            "sentMessageId": message_id,
            "to": to_recipients,
            "cc": cc_recipients,
            "bcc": bcc_recipients,
            "subject": subject,
            "bodySha256": hashlib.sha256(body.encode("utf-8")).hexdigest(),
            "attachmentName": attachment_name or None,
            "attachmentSha256": attachment_sha256 or None,
            "attachmentBytes": attachment_bytes or None,
            "attachmentVerified": bool(attachment_name and attachment_sha256)
            if attachment_name
            else True,
            "sent": True,
            "verified": True,
            "exactMatchCount": 1,
            "uniquenessVerified": True,
            "reusedExisting": reused_existing,
        },
    )


def mail_send_failure(message, code, failure_phase, changed_state):
    return error_result(
        message,
        {
            "status": "error",
            "code": code,
            "failurePhase": failure_phase,
            "sent": False,
            "verified": False,
            "changedState": changed_state,
        },
    )


def mail_send_automation_error_result(error):
    error_type = error.get("error_type") if isinstance(error, dict) else None
    error_number = error.get("error_number") if isinstance(error, dict) else None
    if error_type == "timeout" or error_number == -1712:
        return mail_send_failure(
            "Mail did not confirm Automation access in time. No email was sent.",
            "mail_automation_timeout",
            "preflight",
            "none",
        )
    if error_number == -1743:
        return mail_send_failure(
            "OOMU needs permission to use Mail before it can send this email. Nothing was sent.",
            "mail_automation_permission_required",
            "preflight",
            "none",
        )
    return mail_send_failure(
        "Mail Automation is unavailable right now. No email was sent.",
        "mail_automation_unavailable",
        "preflight",
        "none",
    )


def _legacy_find_exact_mail_draft(to_recipients, cc_recipients, bcc_recipients, subject, body):
    expected_to = ",".join(to_recipients)
    expected_cc = ",".join(cc_recipients)
    expected_bcc = ",".join(bcc_recipients)
    duplicate_sentinel = "OOMU_MULTIPLE_MATCHING_DRAFTS"
    lines = [
        'tell application "Mail"',
        "  set expected_subject to " + applescript_string(subject),
        mail_body_variant_declaration(body),
        "  set expected_to to " + applescript_string(expected_to),
        "  set expected_cc to " + applescript_string(expected_cc),
        "  set expected_bcc to " + applescript_string(expected_bcc),
        "  set matching_messages to {}",
        "  repeat with candidate_message in (every message of drafts mailbox)",
        "    set candidate_body to content of candidate_message as string",
    ]
    lines.extend(mail_body_match_lines("candidate_body"))
    lines.extend(
        [
            "    if (subject of candidate_message as string) is expected_subject and body_matches and my recipient_addresses(to recipients of candidate_message) is expected_to and my recipient_addresses(cc recipients of candidate_message) is expected_cc and my recipient_addresses(bcc recipients of candidate_message) is expected_bcc then",
            "      set end of matching_messages to candidate_message",
            "    end if",
            "  end repeat",
            "  if (count of matching_messages) is greater than 1 then return "
            + applescript_string(duplicate_sentinel),
            "  if (count of matching_messages) is 0 then return \"\"",
            "  set existing_message to item 1 of matching_messages",
            "  return id of existing_message as string",
            "end tell",
            "on recipient_addresses(recipients_list)",
            "  tell application \"Mail\"",
            "    set collected to {}",
            "    repeat with recipient_ref in recipients_list",
            "      set recipient_value to contents of recipient_ref",
            "      set end of collected to (get address of recipient_value) as string",
            "    end repeat",
            "    set AppleScript's text item delimiters to " + applescript_string(","),
            "    set joined to collected as string",
            "    set AppleScript's text item delimiters to \"\"",
            "    return joined",
            "  end tell",
            "end recipient_addresses",
        ]
    )
    script = "\n".join(lines)
    output = run_applescript(script, timeout=UI_AUTOMATION_TIMEOUT_SECONDS).strip()
    error = applescript_tool_error_result(output)
    if error:
        return {"error": "Mail could not inspect existing drafts safely."}
    if output == duplicate_sentinel:
        return {
            "error": "Mail contains multiple identical unsent drafts. Review those drafts before retrying; OOMU did not create another.",
            "residual": True,
        }
    return {"draftId": output} if output else {}


def _legacy_inspect_exact_mail_draft(to_recipients, cc_recipients, bcc_recipients, subject, body):
    expected_to = ",".join(to_recipients)
    expected_cc = ",".join(cc_recipients)
    expected_bcc = ",".join(bcc_recipients)
    duplicate_sentinel = "OOMU_MULTIPLE_MATCHING_DRAFTS"
    lines = [
        'tell application "Mail"',
        "  set expected_subject to " + applescript_string(subject),
        mail_body_variant_declaration(body),
        "  set expected_to to " + applescript_string(expected_to),
        "  set expected_cc to " + applescript_string(expected_cc),
        "  set expected_bcc to " + applescript_string(expected_bcc),
        "  set matching_messages to {}",
        "  repeat with candidate_message in (every message of drafts mailbox)",
        "    set candidate_body to content of candidate_message as string",
    ]
    lines.extend(mail_body_match_lines("candidate_body"))
    lines.extend(
        [
            "    if (subject of candidate_message as string) is expected_subject and body_matches and my recipient_addresses(to recipients of candidate_message) is expected_to and my recipient_addresses(cc recipients of candidate_message) is expected_cc and my recipient_addresses(bcc recipients of candidate_message) is expected_bcc then",
            "      set end of matching_messages to candidate_message",
            "    end if",
            "  end repeat",
            "  if (count of matching_messages) is greater than 1 then return "
            + applescript_string(duplicate_sentinel),
            "  if (count of matching_messages) is 0 then return \"\"",
            "  set verified_message to item 1 of matching_messages",
            "  return id of verified_message as string",
            "end tell",
            "on recipient_addresses(recipients_list)",
            "  tell application \"Mail\"",
            "    set collected to {}",
            "    repeat with recipient_ref in recipients_list",
            "      set recipient_value to contents of recipient_ref",
            "      set end of collected to (get address of recipient_value) as string",
            "    end repeat",
            "    set AppleScript's text item delimiters to " + applescript_string(","),
            "    set joined to collected as string",
            "    set AppleScript's text item delimiters to \"\"",
            "    return joined",
            "  end tell",
            "end recipient_addresses",
        ]
    )
    script = "\n".join(lines)
    output = run_applescript(script, timeout=UI_AUTOMATION_TIMEOUT_SECONDS).strip()
    if applescript_tool_error_result(output):
        return {"error": "Mail could not inspect the final draft safely.", "residual": True}
    if output == duplicate_sentinel:
        return {
            "error": "Mail contains multiple identical unsent drafts.",
            "residual": True,
            "duplicates": True,
        }
    return {"draftId": output} if output else {}


def mail_collision_inventory_lines(collection_expression, result_variable):
    return [
        "  set " + result_variable + " to {}",
        "  repeat with candidate_message in (" + collection_expression + ")",
        "    if (subject of candidate_message as string) is expected_subject and my recipient_addresses(to recipients of candidate_message) is expected_to then",
        "      set end of " + result_variable + " to candidate_message",
        "    end if",
        "  end repeat",
    ]


def mail_exact_inventory_lines(collection_expression, result_variable):
    lines = [
        "  set " + result_variable + " to {}",
        "  repeat with candidate_message in (" + collection_expression + ")",
        "    set candidate_body to content of candidate_message as string",
    ]
    lines.extend(mail_body_match_lines("candidate_body"))
    lines.extend(
        [
            "    if (subject of candidate_message as string) is expected_subject and body_matches and my recipient_addresses(to recipients of candidate_message) is expected_to and my recipient_addresses(cc recipients of candidate_message) is expected_cc and my recipient_addresses(bcc recipients of candidate_message) is expected_bcc then",
            "      set end of " + result_variable + " to candidate_message",
            "    end if",
            "  end repeat",
        ]
    )
    return lines


def authoritative_exact_mail_inventory(
    to_recipients, cc_recipients, bcc_recipients, subject, body
):
    duplicate_sentinel = "OOMU_MULTIPLE_MATCHING_DRAFTS"
    draft_collision_sentinel = "OOMU_MATCHING_DRAFT_COLLISION"
    sent_sentinel = "OOMU_MATCHING_SENT_MESSAGE"
    lines = [
        'tell application "Mail"',
        "  set expected_subject to " + applescript_string(subject),
        mail_body_variant_declaration(body),
        "  set expected_to to " + applescript_string(",".join(to_recipients)),
        "  set expected_cc to " + applescript_string(",".join(cc_recipients)),
        "  set expected_bcc to " + applescript_string(",".join(bcc_recipients)),
    ]
    lines.extend(
        mail_collision_inventory_lines(
            "every message of drafts mailbox whose subject is expected_subject",
            "matching_drafts",
        )
    )
    lines.extend(
        mail_collision_inventory_lines(
            "every message of sent mailbox whose subject is expected_subject",
            "matching_sent",
        )
    )
    lines.extend(mail_exact_inventory_lines("matching_drafts", "exact_drafts"))
    lines.extend(
        [
            "  if (count of matching_sent) is greater than 0 then return "
            + applescript_string(sent_sentinel),
            "  if (count of matching_drafts) is greater than 1 then return "
            + applescript_string(duplicate_sentinel),
            "  if (count of matching_drafts) is 0 then return \"\"",
            "  if (count of exact_drafts) is not 1 then return "
            + applescript_string(draft_collision_sentinel),
            "  return id of item 1 of exact_drafts as string",
            "end tell",
            "on recipient_addresses(recipients_list)",
            '  tell application "Mail"',
            "    set collected to {}",
            "    repeat with recipient_ref in recipients_list",
            "      set recipient_value to contents of recipient_ref",
            "      set end of collected to (get address of recipient_value) as string",
            "    end repeat",
            "    set AppleScript's text item delimiters to " + applescript_string(","),
            "    set joined to collected as string",
            "    set AppleScript's text item delimiters to \"\"",
            "    return joined",
            "  end tell",
            "end recipient_addresses",
        ]
    )
    output = run_applescript(
        "\n".join(lines), timeout=UI_AUTOMATION_TIMEOUT_SECONDS
    ).strip()
    if applescript_tool_error_result(output):
        return {
            "error": "Mail could not verify the final saved and sent-message state safely.",
            "residual": True,
        }
    if output == sent_sentinel:
        return {
            "error": "Mail contains a sent message with this subject and To recipient list.",
            "residual": True,
            "sent": True,
        }
    if output == duplicate_sentinel:
        return {
            "error": "Mail contains multiple unsent drafts with this subject and To recipient list.",
            "residual": True,
            "duplicates": True,
        }
    if output == draft_collision_sentinel:
        return {
            "error": "Mail contains an unsent draft with this subject and To recipient list, but its content or other recipients differ.",
            "residual": True,
            "collision": True,
        }
    return {"draftId": output} if output else {}


def find_exact_mail_draft(to_recipients, cc_recipients, bcc_recipients, subject, body):
    return authoritative_exact_mail_inventory(
        to_recipients, cc_recipients, bcc_recipients, subject, body
    )


def inspect_exact_mail_draft(to_recipients, cc_recipients, bcc_recipients, subject, body):
    return authoritative_exact_mail_inventory(
        to_recipients, cc_recipients, bcc_recipients, subject, body
    )


def inspect_mail_draft_by_subject_token(operation_token):
    duplicate_sentinel = "OOMU_MULTIPLE_TOKEN_DRAFTS"
    state_sentinel = "OOMU_TOKEN_STATE"
    script = "\n".join(
        [
            'tell application "Mail"',
            "  set target_subject to " + applescript_string(operation_token),
            "  set matching_drafts to every message of drafts mailbox whose subject is target_subject",
            "  set matching_outgoing to every outgoing message whose subject is target_subject",
            "  if (count of matching_drafts) is greater than 1 or (count of matching_outgoing) is greater than 1 then return "
            + applescript_string(duplicate_sentinel),
            "  set saved_id to \"\"",
            "  if (count of matching_drafts) is 1 then set saved_id to id of item 1 of matching_drafts as string",
            "  return "
            + applescript_string(state_sentinel)
            + " & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_id & "
            + applescript_string(FIELD_SEPARATOR)
            + " & ((count of matching_outgoing) as string)",
            "end tell",
        ]
    )
    output = run_applescript(script, timeout=UI_AUTOMATION_TIMEOUT_SECONDS).strip()
    if applescript_tool_error_result(output):
        return {
            "error": "Mail could not inspect the prepared draft safely.",
            "residual": True,
        }
    if output == duplicate_sentinel:
        return {
            "error": "Mail contains multiple prepared drafts for this operation.",
            "residual": True,
            "duplicates": True,
        }
    fields = output.split(FIELD_SEPARATOR)
    if (
        len(fields) != 3
        or fields[0] != state_sentinel
        or fields[2].strip() not in {"0", "1"}
    ):
        return {
            "error": "Mail returned an unverified prepared-draft state.",
            "residual": True,
        }
    draft_id = fields[1]
    outgoing_count = fields[2]
    return {
        "draftId": draft_id,
        "outgoingCount": int(outgoing_count.strip()),
        "present": bool(draft_id) or outgoing_count.strip() == "1",
    }


def wait_for_mail_draft_subject_token(operation_token):
    last_draft_id = None
    consecutive_matches = 0
    last_error = None
    for attempt in range(MAIL_DRAFT_SETTLE_ATTEMPTS):
        if attempt > 0:
            time.sleep(MAIL_DRAFT_SETTLE_INTERVAL_SECONDS)
        inspected = inspect_mail_draft_by_subject_token(operation_token)
        if inspected.get("duplicates"):
            return inspected
        if inspected.get("error"):
            last_error = inspected
            last_draft_id = None
            consecutive_matches = 0
            continue
        draft_id = inspected.get("draftId")
        if draft_id:
            if draft_id == last_draft_id:
                consecutive_matches += 1
            else:
                last_draft_id = draft_id
                consecutive_matches = 1
            if consecutive_matches >= 2:
                return {"draftId": draft_id, "settled": True}
        else:
            last_draft_id = None
            consecutive_matches = 0
    return last_error or {}


def wait_for_mail_draft_subject_token_absence(operation_token):
    consecutive_absent = 0
    last_error = None
    for attempt in range(MAIL_DRAFT_SETTLE_ATTEMPTS):
        if attempt > 0:
            time.sleep(MAIL_DRAFT_SETTLE_INTERVAL_SECONDS)
        inspected = inspect_mail_draft_by_subject_token(operation_token)
        if inspected.get("error"):
            last_error = inspected
            consecutive_absent = 0
            continue
        if inspected.get("present"):
            consecutive_absent = 0
        else:
            consecutive_absent += 1
            if consecutive_absent >= 2:
                return {"absent": True, "settled": True}
    return last_error or {
        "error": "Mail retained the prepared-draft operation token.",
        "residual": True,
    }


def wait_for_exact_mail_draft(
    to_recipients,
    cc_recipients,
    bcc_recipients,
    subject,
    body,
    expected_draft_id=None,
    initial_draft_id=None,
):
    last_draft_id = initial_draft_id
    consecutive_matches = 1 if initial_draft_id else 0
    last_error = None
    for attempt in range(MAIL_DRAFT_SETTLE_ATTEMPTS):
        if attempt > 0 or initial_draft_id:
            time.sleep(MAIL_DRAFT_SETTLE_INTERVAL_SECONDS)
        inspected = inspect_exact_mail_draft(
            to_recipients,
            cc_recipients,
            bcc_recipients,
            subject,
            body,
        )
        if (
            inspected.get("duplicates")
            or inspected.get("sent")
            or inspected.get("collision")
        ):
            return inspected
        if inspected.get("error"):
            last_error = inspected
            last_draft_id = None
            consecutive_matches = 0
            continue
        draft_id = inspected.get("draftId")
        if draft_id and (expected_draft_id is None or draft_id == expected_draft_id):
            if draft_id == last_draft_id:
                consecutive_matches += 1
            else:
                last_draft_id = draft_id
                consecutive_matches = 1
            if consecutive_matches >= 2:
                return {"draftId": draft_id, "settled": True}
        else:
            if draft_id:
                last_error = {
                    "error": "Mail contains a matching unsent draft with an unexpected identity.",
                    "residual": True,
                }
            last_draft_id = None
            consecutive_matches = 0
    return last_error or {}


def normalize_mail_body_newlines(body):
    return body.replace("\r\n", "\n").replace("\r", "\n")


def mail_native_blank_paragraph_variant(body, separator):
    normalized = normalize_mail_body_newlines(body)
    rendered = []
    for index, character in enumerate(normalized):
        rendered.append(character)
        if (
            character == "\n"
            and index + 1 < len(normalized)
            and normalized[index + 1] == "\n"
        ):
            rendered.append(separator)
    return "".join(rendered)


def mail_body_readback_variants(requested_body):
    requested = normalize_mail_body_newlines(requested_body)
    if len(requested) > MAX_TEXT_CHARS:
        return ()
    bases = [
        requested,
        mail_native_blank_paragraph_variant(requested, "\u2028"),
        mail_native_blank_paragraph_variant(requested, "\u2029"),
    ]
    variants = []
    for base in bases:
        for prefix in ("", "\n"):
            for suffix in ("", " ", " \n"):
                candidate = prefix + base + suffix
                if candidate not in variants:
                    variants.append(candidate)
    return tuple(variants)


def mail_body_variant_declaration(requested_body, indent="  "):
    variants = mail_body_readback_variants(requested_body)
    return (
        indent
        + "set expected_body_variants to {"
        + ", ".join(applescript_string(variant) for variant in variants)
        + "}"
    )


def mail_body_match_lines(candidate_variable, indent="    "):
    return [
        indent + "set body_matches to false",
        indent + "repeat with expected_body_ref in expected_body_variants",
        indent
        + "  if "
        + candidate_variable
        + " is (contents of expected_body_ref) then",
        indent + "    set body_matches to true",
        indent + "    exit repeat",
        indent + "  end if",
        indent + "end repeat",
    ]


def mail_body_readback_matches(requested_body, saved_body):
    saved = normalize_mail_body_newlines(saved_body)
    variants = mail_body_readback_variants(requested_body)
    if not variants:
        return False
    if len(saved) > max(len(variant) for variant in variants):
        return False
    return saved in variants


def mail_cleanup_result(state):
    return {
        "state": state,
        "verified": state in {"absent", "neutralized"},
        "residual": state != "absent",
    }


def mail_inventory_failure(message, code, failure_phase, residual_draft_possible):
    return error_result(
        message,
        {
            "status": "error",
            "code": code,
            "failurePhase": failure_phase,
            "saved": False,
            "verified": False,
            "cleanupState": "not_required",
            "cleanupVerified": True,
            "residualDraftPossible": residual_draft_possible,
        },
    )


def mail_draft_success_result(
    draft_id,
    to_recipients,
    cc_recipients,
    bcc_recipients,
    subject,
    body,
    reused_existing,
):
    return bool_result(
        True,
        {
            "draftId": draft_id,
            "to": to_recipients,
            "cc": cc_recipients,
            "bcc": bcc_recipients,
            "subject": subject,
            "bodySha256": hashlib.sha256(body.encode("utf-8")).hexdigest(),
            "draftState": "outgoing_message",
            "sent": False,
            "saved": True,
            "verified": True,
            "exactMatchCount": 1,
            "uniquenessVerified": True,
            "reusedExisting": reused_existing,
        },
    )


def mail_draft_failure(
    message,
    cleanup_result,
    failure_phase,
    force_residual=False,
):
    if not isinstance(cleanup_result, dict):
        cleanup_result = mail_cleanup_result("unverified")
    cleanup_state = cleanup_result.get("state", "unverified")
    cleanup_verified = cleanup_result.get("verified") is True
    residual_draft_possible = force_residual or cleanup_result.get("residual") is True
    if cleanup_state == "unverified":
        code = "mail_draft_result_unverified"
    elif residual_draft_possible:
        code = "mail_draft_review_required"
    else:
        code = "mail_draft_creation_failed_cleanly"
    if cleanup_state == "absent":
        cleanup_message = " The unverified draft is no longer present."
    elif cleanup_state == "neutralized":
        cleanup_message = (
            " Mail retained a closed draft object, but OOMU verified that its recipients and body "
            "were cleared and its subject was retagged for safe review."
        )
    else:
        cleanup_message = " Review Mail before retrying."
    return error_result(
        message + cleanup_message,
        {
            "status": "error",
            "code": code,
            "failurePhase": failure_phase,
            "saved": False,
            "verified": False,
            "cleanupState": cleanup_state,
            "cleanupVerified": cleanup_verified,
            "residualDraftPossible": residual_draft_possible,
        },
    )


def _legacy_remove_mail_draft_by_id(draft_id):
    cleanup_subject = "OOMU-CLEANUP-" + secrets.token_hex(16)
    script = "\n".join(
        [
            'tell application "Mail"',
            "  set target_id to " + applescript_string(draft_id),
            "  set cleanup_subject to " + applescript_string(cleanup_subject),
            "  set matching_messages to {}",
            "  repeat with candidate_message in (every outgoing message)",
            "    if (id of candidate_message as string) is target_id then set end of matching_messages to candidate_message",
            "  end repeat",
            "  set original_count to count of matching_messages",
            "  if original_count is 1 then",
            "    try",
            "      delete item 1 of matching_messages",
            "    end try",
            "  end if",
            "  delay " + str(MAIL_CLEANUP_SETTLE_SECONDS),
            "  set matching_messages to {}",
            "  repeat with candidate_message in (every outgoing message)",
            "    if (id of candidate_message as string) is target_id then set end of matching_messages to candidate_message",
            "  end repeat",
            "  if (count of matching_messages) is 1 then",
            "    set target_message to item 1 of matching_messages",
            "    try",
            "      tell target_message to delete every to recipient",
            "    end try",
            "    try",
            "      tell target_message to delete every cc recipient",
            "    end try",
            "    try",
            "      tell target_message to delete every bcc recipient",
            "    end try",
            "    try",
            "      set content of target_message to \"\"",
            "    end try",
            "    try",
            "      set subject of target_message to cleanup_subject",
            "    end try",
            "    try",
            "      set visible of target_message to false",
            "    end try",
            "    try",
            "      save target_message",
            "    end try",
            "  end if",
            "  delay " + str(MAIL_CLEANUP_SETTLE_SECONDS),
            "  set matching_messages to {}",
            "  repeat with candidate_message in (every outgoing message)",
            "    if (id of candidate_message as string) is target_id then set end of matching_messages to candidate_message",
            "  end repeat",
            "  if (count of matching_messages) is 1 then",
            "    try",
            "      delete item 1 of matching_messages",
            "    end try",
            "  end if",
            "  delay " + str(MAIL_CLEANUP_SETTLE_SECONDS),
            "  set matching_messages to {}",
            "  repeat with candidate_message in (every outgoing message)",
            "    if (id of candidate_message as string) is target_id then set end of matching_messages to candidate_message",
            "  end repeat",
            "  set remaining_count to count of matching_messages",
            "  set saved_subject to \"\"",
            "  set saved_body to \"\"",
            "  set saved_to to \"\"",
            "  set saved_cc to \"\"",
            "  set saved_bcc to \"\"",
            "  set saved_visible to \"\"",
            "  set saved_class to \"\"",
            "  if remaining_count is 1 then",
            "    set target_message to item 1 of matching_messages",
            "    set saved_subject to subject of target_message as string",
            "    set saved_body to content of target_message as string",
            "    set saved_to to my recipient_addresses(to recipients of target_message)",
            "    set saved_cc to my recipient_addresses(cc recipients of target_message)",
            "    set saved_bcc to my recipient_addresses(bcc recipients of target_message)",
            "    set saved_visible to visible of target_message as string",
            "    set saved_class to class of target_message as string",
            "  end if",
            "  return (original_count as string) & "
            + applescript_string(FIELD_SEPARATOR)
            + " & (remaining_count as string) & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_subject & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_body & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_to & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_cc & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_bcc & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_visible & "
            + applescript_string(FIELD_SEPARATOR)
            + " & saved_class",
            "end tell",
            "on recipient_addresses(recipients_list)",
            "  tell application \"Mail\"",
            "    set collected to {}",
            "    repeat with recipient_ref in recipients_list",
            "      set recipient_value to contents of recipient_ref",
            "      set end of collected to (get address of recipient_value) as string",
            "    end repeat",
            "    set AppleScript's text item delimiters to " + applescript_string(","),
            "    set joined to collected as string",
            "    set AppleScript's text item delimiters to \"\"",
            "    return joined",
            "  end tell",
            "end recipient_addresses",
        ]
    )
    output = run_applescript(script, timeout=UI_AUTOMATION_TIMEOUT_SECONDS)
    if applescript_tool_error_result(output):
        return mail_cleanup_result("unverified")
    fields = output.split(FIELD_SEPARATOR)
    while len(fields) < 9:
        fields.append("")
    original, remaining, saved_subject, saved_body, saved_to, saved_cc, saved_bcc, saved_visible, saved_class = fields[:9]
    if original.strip() in {"0", "1"} and remaining.strip() == "0":
        return mail_cleanup_result("absent")
    neutralized = (
        original.strip() == "1"
        and remaining.strip() == "1"
        and saved_subject == cleanup_subject
        and mail_body_readback_matches("", saved_body)
        and saved_to == ""
        and saved_cc == ""
        and saved_bcc == ""
        and saved_visible.strip().lower() == "false"
        and saved_class.strip().lower() == "outgoing message"
    )
    return mail_cleanup_result("neutralized" if neutralized else "unverified")


def _legacy_remove_mail_draft_by_subject_token(operation_token):
    script = "\n".join(
        [
            'tell application "Mail"',
            "  set target_subject to " + applescript_string(operation_token),
            "  set matching_messages to every outgoing message whose subject is target_subject",
            "  set removed_count to count of matching_messages",
            "  repeat with candidate_message in matching_messages",
            "    delete candidate_message",
            "  end repeat",
            "  delay " + str(MAIL_CLEANUP_SETTLE_SECONDS),
            "  set matching_messages to every outgoing message whose subject is target_subject",
            "  repeat with candidate_message in matching_messages",
            "    delete candidate_message",
            "  end repeat",
            "  delay " + str(MAIL_CLEANUP_SETTLE_SECONDS),
            "  set remaining_count to count of (every outgoing message whose subject is target_subject)",
            "  return (removed_count as string) & "
            + applescript_string(FIELD_SEPARATOR)
            + " & (remaining_count as string)",
            "end tell",
        ]
    )
    output = run_applescript(script, timeout=UI_AUTOMATION_TIMEOUT_SECONDS)
    if applescript_tool_error_result(output):
        return mail_cleanup_result("unverified")
    removed, _, remaining = output.partition(FIELD_SEPARATOR)
    if removed.strip() in {"0", "1"} and remaining.strip() == "0":
        return mail_cleanup_result("absent")
    return mail_cleanup_result("unverified")


def mail_cleanup_inventory_lines(collection_expression, result_variable):
    lines = [
        "  set " + result_variable + " to {}",
        "  repeat with candidate_message in (" + collection_expression + ")",
        "    set target_matches to false",
        "    if target_id is not \"\" then",
        "      if (id of candidate_message as string) is target_id then set target_matches to true",
        "    end if",
        "    if not target_matches and target_subject is not \"\" then",
        "      if (subject of candidate_message as string) is target_subject then set target_matches to true",
        "    end if",
        "    if not target_matches and has_exact_target then",
        "      set candidate_body to content of candidate_message as string",
    ]
    lines.extend(mail_body_match_lines("candidate_body", indent="      "))
    lines.extend(
        [
            "      if (subject of candidate_message as string) is expected_subject and body_matches and my recipient_addresses(to recipients of candidate_message) is expected_to and my recipient_addresses(cc recipients of candidate_message) is expected_cc and my recipient_addresses(bcc recipients of candidate_message) is expected_bcc then set target_matches to true",
            "    end if",
            "    if target_matches then set end of "
            + result_variable
            + " to candidate_message",
            "  end repeat",
        ]
    )
    return lines


def mail_cleanup_delete_lines(result_variable):
    return [
        "  repeat with candidate_message in " + result_variable,
        "    try",
        "      delete candidate_message",
        "    end try",
        "  end repeat",
    ]


def mail_cleanup_collection_expression(
    collection_expression, operation_token, has_exact_target
):
    if operation_token and has_exact_target:
        return (
            collection_expression
            + " whose (subject is target_subject or subject is expected_subject)"
        )
    if operation_token:
        return collection_expression + " whose subject is target_subject"
    if has_exact_target:
        return collection_expression + " whose subject is expected_subject"
    return collection_expression


def remove_mail_draft_by_id(
    draft_id,
    operation_token="",
    to_recipients=None,
    cc_recipients=None,
    bcc_recipients=None,
    subject=None,
    body=None,
):
    has_exact_target = (
        to_recipients is not None
        and cc_recipients is not None
        and bcc_recipients is not None
        and subject is not None
        and body is not None
    )
    to_recipients = [] if to_recipients is None else to_recipients
    cc_recipients = [] if cc_recipients is None else cc_recipients
    bcc_recipients = [] if bcc_recipients is None else bcc_recipients
    subject = "" if subject is None else subject
    body = "" if body is None else body
    drafts_expression = mail_cleanup_collection_expression(
        "every message of drafts mailbox", operation_token, has_exact_target
    )
    outgoing_expression = mail_cleanup_collection_expression(
        "every outgoing message", operation_token, has_exact_target
    )
    sent_expression = mail_cleanup_collection_expression(
        "every message of sent mailbox", operation_token, has_exact_target
    )
    lines = [
        'tell application "Mail"',
        "  set target_id to " + applescript_string(draft_id),
        "  set target_subject to " + applescript_string(operation_token),
        "  set has_exact_target to " + ("true" if has_exact_target else "false"),
        "  set expected_subject to " + applescript_string(subject),
        mail_body_variant_declaration(body),
        "  set expected_to to " + applescript_string(",".join(to_recipients)),
        "  set expected_cc to " + applescript_string(",".join(cc_recipients)),
        "  set expected_bcc to " + applescript_string(",".join(bcc_recipients)),
    ]
    lines.extend(
        mail_cleanup_inventory_lines(drafts_expression, "saved_matches")
    )
    lines.extend(
        mail_cleanup_inventory_lines(outgoing_expression, "outgoing_matches")
    )
    lines.extend(
        mail_cleanup_inventory_lines(sent_expression, "sent_matches")
    )
    lines.extend(
        [
            "  set original_saved_count to count of saved_matches",
            "  set original_outgoing_count to count of outgoing_matches",
            "  set original_sent_count to count of sent_matches",
        ]
    )
    lines.extend(mail_cleanup_delete_lines("outgoing_matches"))
    lines.extend(mail_cleanup_delete_lines("saved_matches"))
    lines.append("  delay " + str(MAIL_CLEANUP_SETTLE_SECONDS))
    lines.extend(
        mail_cleanup_inventory_lines(drafts_expression, "saved_matches")
    )
    lines.extend(
        mail_cleanup_inventory_lines(outgoing_expression, "outgoing_matches")
    )
    lines.extend(mail_cleanup_delete_lines("outgoing_matches"))
    lines.extend(mail_cleanup_delete_lines("saved_matches"))
    lines.append("  delay " + str(MAIL_CLEANUP_SETTLE_SECONDS))
    lines.extend(
        mail_cleanup_inventory_lines(drafts_expression, "saved_matches")
    )
    lines.extend(
        mail_cleanup_inventory_lines(outgoing_expression, "outgoing_matches")
    )
    lines.extend(
        mail_cleanup_inventory_lines(sent_expression, "sent_matches")
    )
    lines.extend(
        [
            "  return (original_saved_count as string) & "
            + applescript_string(FIELD_SEPARATOR)
            + " & (original_outgoing_count as string) & "
            + applescript_string(FIELD_SEPARATOR)
            + " & ((count of saved_matches) as string) & "
            + applescript_string(FIELD_SEPARATOR)
            + " & ((count of outgoing_matches) as string) & "
            + applescript_string(FIELD_SEPARATOR)
            + " & (original_sent_count as string) & "
            + applescript_string(FIELD_SEPARATOR)
            + " & ((count of sent_matches) as string)",
            "end tell",
            "on recipient_addresses(recipients_list)",
            '  tell application "Mail"',
            "    set collected to {}",
            "    repeat with recipient_ref in recipients_list",
            "      set recipient_value to contents of recipient_ref",
            "      set end of collected to (get address of recipient_value) as string",
            "    end repeat",
            "    set AppleScript's text item delimiters to " + applescript_string(","),
            "    set joined to collected as string",
            "    set AppleScript's text item delimiters to \"\"",
            "    return joined",
            "  end tell",
            "end recipient_addresses",
        ]
    )
    output = run_applescript(
        "\n".join(lines), timeout=UI_AUTOMATION_TIMEOUT_SECONDS
    )
    if applescript_tool_error_result(output):
        return mail_cleanup_result("unverified")
    fields = output.split(FIELD_SEPARATOR)
    while len(fields) < 6:
        fields.append("")
    (
        original_saved,
        original_outgoing,
        remaining_saved,
        remaining_outgoing,
        original_sent,
        remaining_sent,
    ) = fields[:6]
    counts_are_verified = all(
        value.strip().isdigit()
        for value in (
            original_saved,
            original_outgoing,
            remaining_saved,
            remaining_outgoing,
            original_sent,
            remaining_sent,
        )
    )
    if (
        counts_are_verified
        and remaining_saved.strip() == "0"
        and remaining_outgoing.strip() == "0"
        and remaining_sent.strip() == "0"
    ):
        return mail_cleanup_result("absent")
    return mail_cleanup_result("unverified")


def remove_mail_draft_by_subject_token(operation_token):
    return remove_mail_draft_by_id("", operation_token=operation_token)


def recipients_arg(arguments, name, required):
    value = text_arg(arguments, name, default="", required=required, max_chars=4096)
    recipients = [item.strip() for item in value.split(",") if item.strip()]
    if required and not recipients:
        raise ToolInputError(name + " must include at least one recipient.")
    for recipient in recipients:
        if "@" not in recipient or any(char.isspace() for char in recipient):
            raise ToolInputError("Invalid email address in " + name + ".")
    return recipients


def recipient_lines(kind, recipients):
    return [
        "    make new "
        + kind
        + " at end of "
        + kind
        + "s with properties {address:"
        + applescript_string(recipient)
        + "}"
        for recipient in recipients
    ]


def create_system_note(arguments):
    folder_name = text_arg(arguments, "folder_name", default="", max_chars=256).strip()
    title = text_arg(arguments, "title", required=True, max_chars=512)
    body = text_arg(arguments, "body", required=True)

    target_line = (
        "  set target_folder to folder " + applescript_string(folder_name)
        if folder_name
        else "  set target_folder to default folder"
    )
    script = "\n".join(
        [
            "tell application \"Notes\"",
            "  try",
            target_line,
            "  on error",
            "    set target_folder to folder \"Notes\"",
            "  end try",
            "  set new_note to make new note at target_folder with properties {name:"
            + applescript_string(title)
            + ", body:"
            + applescript_string(body)
            + "}",
            "  return (id of new_note as string) & tab & (name of new_note as string)",
            "end tell",
        ]
    )

    output = run_applescript(script)
    error = applescript_tool_error_result(output)
    if error:
        return error
    note_id, _, note_title = output.partition("\t")
    return text_result(
        json.dumps({"id": note_id, "title": note_title or title}, indent=2),
        {
            "id": note_id,
            "title": note_title or title,
            "folderName": folder_name,
        },
    )


def read_system_emails(arguments):
    max_messages = int(number_arg(arguments, "max_messages", 10, 1, 50))
    unread_only = bool(arguments.get("unread_only", False))
    if not preflight_mail_automation():
        return permission_blocked_or_timed_out_result()

    message_collection_lines = []
    if unread_only:
        message_collection_lines = [
            "  set inbox_messages to every message of inbox whose read status is false",
            "  set message_count to count of inbox_messages",
            "  if message_count is greater than 0 then",
            "    if message_count is less than max_messages then set max_messages to message_count",
            "    repeat with index from 1 to max_messages",
            "      set message_item to item index of inbox_messages",
        ]
    else:
        message_collection_lines = [
            "  set message_count to count of messages of inbox",
            "  if message_count is greater than 0 then",
            "    if message_count is less than max_messages then set max_messages to message_count",
            "    repeat with index from 1 to max_messages",
            "      set message_item to message index of inbox",
        ]

    script = "\n".join(
        [
            "set field_separator to ASCII character 31",
            "set record_separator to ASCII character 30",
            "set output_rows to {}",
            "tell application \"Mail\"",
            "    set max_messages to " + str(max_messages),
            *message_collection_lines,
            "      set message_subject to subject of message_item",
            "      if message_subject is missing value then set message_subject to \"\"",
            "      set message_sender to sender of message_item",
            "      if message_sender is missing value then set message_sender to \"\"",
            "      set message_date to date received of message_item as string",
            "      set message_content to content of message_item",
            "      if message_content is missing value then set message_content to \"\"",
            "      if length of message_content is greater than 500 then",
            "        set message_content to text 1 thru 500 of message_content",
            "      end if",
            "      set message_read to read status of message_item",
            "      set read_str to \"true\"",
            "      if message_read is false then set read_str to \"false\"",
            "      set end of output_rows to (message_sender & field_separator & message_subject & field_separator & message_date & field_separator & read_str & field_separator & message_content)",
            "    end repeat",
            "  end if",
            "end tell",
            "set AppleScript's text item delimiters to record_separator",
            "return output_rows as text",
        ]
    )

    output = run_applescript(script, timeout=MAIL_READ_TIMEOUT_SECONDS)
    error = parse_applescript_error(output)
    if error:
        return degraded_collection_result(
            "emails",
            {
                "maxMessages": max_messages,
                "unreadOnly": unread_only,
            },
            error,
        )
    emails = parse_email_rows(output)
    return text_result(
        json.dumps(emails, indent=2),
        {
            "maxMessages": max_messages,
            "unreadOnly": unread_only,
            "emails": emails,
        },
    )


def parse_email_rows(output):
    if output.strip() == "":
        return []
    emails = []
    for row in output.split(RECORD_SEPARATOR):
        if not row:
            continue
        columns = row.split(FIELD_SEPARATOR)
        while len(columns) < 5:
            columns.append("")
        emails.append(
            {
                "sender": columns[0],
                "subject": columns[1],
                "dateReceived": columns[2],
                "read": columns[3] == "true",
                "content": columns[4],
            }
        )
    return emails


def read_system_notes(arguments):
    max_notes = int(number_arg(arguments, "max_notes", 20, 1, 50))
    search_text = text_arg(arguments, "search_text", default="", max_chars=512).strip().lower()
    include_body = bool(arguments.get("include_body", True))

    script = "\n".join(
        [
            "set field_separator to ASCII character 31",
            "set record_separator to ASCII character 30",
            "set output_rows to {}",
            "tell application \"Notes\"",
            "  set note_items to every note",
            "  repeat with note_item in note_items",
            "    set note_name to name of note_item",
            "    if note_name is missing value then set note_name to \"\"",
            "    set note_created to creation date of note_item as string",
            "    set note_modified to modification date of note_item as string",
            "    set note_body to body of note_item",
            "    if note_body is missing value then set note_body to \"\"",
            "    if length of note_body is greater than 1200 then",
            "      set note_body to text 1 thru 1200 of note_body",
            "    end if",
            "    set end of output_rows to ((note_name as string) & field_separator & note_created & field_separator & note_modified & field_separator & (note_body as string))",
            "  end repeat",
            "end tell",
            "set AppleScript's text item delimiters to record_separator",
            "return output_rows as text",
        ]
    )

    output = run_applescript(script)
    error = parse_applescript_error(output)
    if error:
        return degraded_collection_result(
            "notes",
            {
                "maxNotes": max_notes,
                "searchText": search_text,
                "includeBody": include_body,
            },
            error,
        )
    notes = parse_note_rows(output, include_body)
    if search_text:
        notes = [
            note
            for note in notes
            if search_text in (note.get("title", "") + " " + note.get("body", "")).lower()
        ]
    notes = notes[:max_notes]
    return text_result(
        json.dumps(notes, indent=2),
        {
            "maxNotes": max_notes,
            "searchText": search_text,
            "includeBody": include_body,
            "notes": notes,
        },
    )


def parse_note_rows(output, include_body):
    if output.strip() == "":
        return []
    notes = []
    for row in output.split(RECORD_SEPARATOR):
        if not row:
            continue
        columns = row.split(FIELD_SEPARATOR)
        while len(columns) < 4:
            columns.append("")
        note = {
            "title": columns[0],
            "createdAt": columns[1],
            "updatedAt": columns[2],
        }
        if include_body:
            note["body"] = columns[3]
        notes.append(note)
    return notes


def read_system_contacts(arguments):
    max_contacts = int(number_arg(arguments, "max_contacts", 20, 1, 50))
    search_text = text_arg(arguments, "search_text", default="", max_chars=512).strip()
    search_detail_fields = any(character.isdigit() or character == "@" for character in search_text)

    script = "\n".join(
        [
            "set field_separator to ASCII character 31",
            "set record_separator to ASCII character 30",
            "set search_query to " + applescript_string(search_text),
            "set search_detail_fields to " + ("true" if search_detail_fields else "false"),
            "set max_matches to " + str(max_contacts),
            "set output_rows to {}",
            "with timeout of " + str(DEFAULT_TIMEOUT_SECONDS) + " seconds",
            "tell application \"Contacts\"",
            "  set people_items to every person",
            "  repeat with person_item in people_items",
            "    set first_name to first name of person_item",
            "    if first_name is missing value then set first_name to \"\"",
            "    set last_name to last name of person_item",
            "    if last_name is missing value then set last_name to \"\"",
            "    set org_name to organization of person_item",
            "    if org_name is missing value then set org_name to \"\"",
            "    set contact_matches to false",
            "    if search_query is \"\" then set contact_matches to true",
            "    if not contact_matches then",
            "      set contact_search_text to ((first_name as string) & \" \" & (last_name as string) & \" \" & (org_name as string))",
            "      ignoring case",
            "        if contact_search_text contains search_query then set contact_matches to true",
            "      end ignoring",
            "    end if",
            "    set email_text to \"\"",
            "    set phone_text to \"\"",
            "    if contact_matches then",
            "      set email_values to {}",
            "      repeat with email_item in emails of person_item",
            "        set end of email_values to value of email_item as string",
            "      end repeat",
            "      set phone_values to {}",
            "      repeat with phone_item in phones of person_item",
            "        set end of phone_values to value of phone_item as string",
            "      end repeat",
            "      set AppleScript's text item delimiters to \", \"",
            "      set email_text to email_values as text",
            "      set phone_text to phone_values as text",
            "      set AppleScript's text item delimiters to \"\"",
            "    else if search_detail_fields then",
            "      set email_values to {}",
            "      repeat with email_item in emails of person_item",
            "        set end of email_values to value of email_item as string",
            "      end repeat",
            "      set phone_values to {}",
            "      repeat with phone_item in phones of person_item",
            "        set end of phone_values to value of phone_item as string",
            "      end repeat",
            "      set AppleScript's text item delimiters to \", \"",
            "      set email_text to email_values as text",
            "      set phone_text to phone_values as text",
            "      set AppleScript's text item delimiters to \"\"",
            "      set contact_detail_text to (email_text & \" \" & phone_text)",
            "      ignoring case",
            "        if contact_detail_text contains search_query then set contact_matches to true",
            "      end ignoring",
            "    end if",
            "    if contact_matches then",
            "      set end of output_rows to ((first_name as string) & field_separator & (last_name as string) & field_separator & (org_name as string) & field_separator & email_text & field_separator & phone_text)",
            "      if (count of output_rows) is greater than or equal to max_matches then exit repeat",
            "    end if",
            "  end repeat",
            "end tell",
            "end timeout",
            "set AppleScript's text item delimiters to record_separator",
            "return output_rows as text",
        ]
    )

    output = run_applescript(script)
    error = parse_applescript_error(output)
    if error:
        return degraded_collection_result(
            "contacts",
            {
                "maxContacts": max_contacts,
                "searchText": search_text,
            },
            error,
        )
    contacts = parse_contact_rows(output)[:max_contacts]
    return text_result(
        json.dumps(contacts, indent=2),
        {
            "maxContacts": max_contacts,
            "searchText": search_text,
            "contacts": contacts,
        },
    )


def parse_contact_rows(output):
    if output.strip() == "":
        return []
    contacts = []
    for row in output.split(RECORD_SEPARATOR):
        if not row:
            continue
        columns = row.split(FIELD_SEPARATOR)
        while len(columns) < 5:
            columns.append("")
        name = " ".join([columns[0].strip(), columns[1].strip()]).strip()
        contacts.append(
            {
                "name": name,
                "organization": columns[2],
                "emails": [item.strip() for item in columns[3].split(",") if item.strip()],
                "phones": [item.strip() for item in columns[4].split(",") if item.strip()],
            }
        )
    return contacts


def read_system_reminders(arguments):
    list_name = text_arg(arguments, "list_name", default="", max_chars=256).strip()
    completed_only = bool(arguments.get("completed_only", False))
    if not preflight_reminders_automation():
        return permission_blocked_or_timed_out_result()

    completed_clause = "whose completed is false"
    if completed_only:
        completed_clause = "whose completed is true"

    target_lists_line = (
        "  set target_lists to {list " + applescript_string(list_name) + "}"
        if list_name
        else "  set target_lists to lists"
    )

    script = "\n".join(
        [
            "set field_separator to ASCII character 31",
            "set record_separator to ASCII character 30",
            "set output_rows to {}",
            "tell application \"Reminders\"",
            target_lists_line,
            "  repeat with target_list in target_lists",
            "    tell target_list",
            "    set target_list_name to name of it",
            "    set matching_reminders to reminders of it " + completed_clause,
            "    repeat with reminder_item in matching_reminders",
            "      set reminder_name to name of reminder_item",
            "      if reminder_name is missing value then set reminder_name to \"\"",
            "      set reminder_body to body of reminder_item",
            "      if reminder_body is missing value then set reminder_body to \"\"",
            "      set reminder_due to due date of reminder_item",
            "      set due_str to \"\"",
            "      if reminder_due is not missing value then set due_str to reminder_due as string",
            "      set end of output_rows to (target_list_name & field_separator & reminder_name & field_separator & reminder_body & field_separator & due_str)",
            "    end repeat",
            "    end tell",
            "  end repeat",
            "end tell",
            "set AppleScript's text item delimiters to record_separator",
            "return output_rows as text",
        ]
    )

    output = run_applescript(script)
    error = parse_applescript_error(output)
    if error:
        return degraded_collection_result(
            "reminders",
            {
                "listName": list_name,
                "completedOnly": completed_only,
            },
            error,
        )
    reminders = parse_reminder_rows(output)
    return text_result(
        json.dumps(reminders, indent=2),
        {
            "listName": list_name,
            "completedOnly": completed_only,
            "reminders": reminders,
        },
    )


def parse_reminder_rows(output):
    if output.strip() == "":
        return []
    reminders = []
    for row in output.split(RECORD_SEPARATOR):
        if not row:
            continue
        columns = row.split(FIELD_SEPARATOR)
        original_column_count = len(columns)
        while len(columns) < 4:
            columns.append("")
        if original_column_count == 3:
            columns = ["", columns[0], columns[1], columns[2]]
        reminders.append(
            {
                "listName": columns[0],
                "title": columns[1],
                "body": columns[2],
                "dueDate": columns[3],
            }
        )
    return reminders


def canonical_apple_app_name(value):
    requested = text_arg({"app_name": value}, "app_name", required=True, max_chars=128)
    normalized = " ".join(requested.strip().lower().split())
    app_name = APPLE_APP_ALIASES.get(normalized)
    if not app_name:
        raise ToolInputError("Unsupported Apple app for UI reading: " + requested)
    return app_name


def read_apple_app_ui(arguments):
    app_name = canonical_apple_app_name(arguments.get("app_name"))
    max_items = int(number_arg(arguments, "max_items", 80, 1, MAX_UI_TEXT_ITEMS))
    activate = bool(arguments.get("activate", True))

    lines = [
        "set field_separator to ASCII character 31",
        "set record_separator to ASCII character 30",
        "set output_rows to {}",
    ]
    if activate:
        lines.extend(
            [
                "tell application " + applescript_string(app_name),
                "  activate",
                "end tell",
                "delay 0.5",
            ]
        )
    lines.extend(
        [
            "with timeout of " + str(UI_AUTOMATION_TIMEOUT_SECONDS) + " seconds",
            "tell application \"System Events\"",
            "  tell process " + applescript_string(app_name),
            "    if not (exists window 1) then return \"\"",
            "    set ui_items to entire contents of window 1",
            "    repeat with ui_item in ui_items",
            "      set item_text to \"\"",
            "      try",
            "        set item_text to value of ui_item as string",
            "      end try",
            "      if item_text is \"\" then",
            "        try",
            "          set item_text to name of ui_item as string",
            "        end try",
            "      end if",
            "      if item_text is not \"\" then",
            "        set end of output_rows to item_text",
            "        if (count of output_rows) is greater than or equal to " + str(max_items) + " then exit repeat",
            "      end if",
            "    end repeat",
            "  end tell",
            "end tell",
            "end timeout",
            "set AppleScript's text item delimiters to record_separator",
            "return output_rows as text",
        ]
    )

    output = run_applescript("\n".join(lines), timeout=UI_AUTOMATION_TIMEOUT_SECONDS)
    error = parse_applescript_error(output)
    if error:
        return degraded_collection_result(
            "uiText",
            {
                "appName": app_name,
                "maxItems": max_items,
                "activated": activate,
            },
            error,
        )
    items = parse_ui_text_rows(output, max_items)
    return text_result(
        json.dumps(items, indent=2),
        {
            "appName": app_name,
            "maxItems": max_items,
            "activated": activate,
            "uiText": items,
        },
    )


def parse_ui_text_rows(output, max_items):
    if output.strip() == "":
        return []
    items = []
    seen = set()
    for row in output.split(RECORD_SEPARATOR):
        text = " ".join(row.split())
        if not text or text in seen:
            continue
        seen.add(text)
        if len(text) > 500:
            text = text[:500]
        items.append(text)
        if len(items) >= max_items:
            break
    return items


def call_tool(params):
    name = params.get("name")
    arguments = params.get("arguments") or {}
    if not isinstance(arguments, dict):
        return error_result("Tool arguments must be an object.")

    try:
        if name == "trigger_system_notification":
            return trigger_system_notification(arguments)
        if name == "read_system_calendar":
            return read_system_calendar(arguments)
        if name == "add_system_reminder":
            return add_system_reminder(arguments)
        if name == "draft_system_email":
            return draft_system_email(arguments)
        if name == "prepare_system_message":
            return prepare_system_message(arguments)
        if name == "capture_disposable_window":
            return error_result(
                "Screen capture requires OOMU's native app boundary.",
                {"code": "screen_capture_native_boundary_required", "verified": False},
            )
        if name == "preview_camera":
            return error_result(
                "Camera preview requires OOMU's native app boundary.",
                {"code": "camera_preview_native_boundary_required", "verified": False},
            )
        if name in {"read_system_music", "read_system_photos"}:
            return error_result(
                "This read requires OOMU's native app boundary.",
                {"code": "native_read_boundary_required", "verified": False},
            )
        if name == "send_system_email":
            return send_system_email(arguments)
        if name == "create_system_note":
            return create_system_note(arguments)
        if name == "read_system_emails":
            return read_system_emails(arguments)
        if name == "read_system_notes":
            return read_system_notes(arguments)
        if name == "read_system_contacts":
            return read_system_contacts(arguments)
        if name == "read_system_reminders":
            return read_system_reminders(arguments)
        if name == "read_apple_app_ui":
            return read_apple_app_ui(arguments)
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
