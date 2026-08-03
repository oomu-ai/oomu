use super::{
    opaque_id, BrowserSession, BrowserSnapshot, SemanticNode, MAX_ACTION_TEXT_CHARS,
    MAX_SEMANTIC_NODES,
};
use serde::Deserialize;
use std::time::Duration;
use tauri::Manager;

const BROWSER_WEBVIEW_LABEL: &str = "oomu-browser-mod";
const DRIVER_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug)]
pub(super) struct ElementTarget {
    pub(super) path: Vec<i64>,
    pub(super) marker: String,
    pub(super) document_marker: String,
    pub(super) role: String,
    pub(super) name: String,
    pub(super) value_class: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSnapshot {
    document_marker: String,
    url: String,
    title: String,
    nodes: Vec<RawNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawNode {
    role: String,
    name: String,
    value_class: String,
    visible: bool,
    enabled: bool,
    path: Vec<i64>,
    marker: String,
}

pub(super) async fn snapshot(
    app: &tauri::AppHandle,
    session: &mut BrowserSession,
) -> Result<BrowserSnapshot, String> {
    let document_marker = opaque_id("docmark");
    let script = snapshot_script(
        &session.document_marker_key,
        &session.element_marker_key,
        &document_marker,
    )?;
    let raw: RawSnapshot = evaluate_json(app, script).await?;
    let changed = session
        .current_document_marker
        .as_ref()
        .is_some_and(|current| current != &raw.document_marker);
    if changed || session.current_document_marker.is_none() {
        session.document_generation = session.document_generation.saturating_add(1);
        session.references.clear();
    }
    session.current_document_marker = Some(raw.document_marker.clone());
    let mut nodes = Vec::with_capacity(raw.nodes.len().min(MAX_SEMANTIC_NODES));
    for node in raw.nodes.into_iter().take(MAX_SEMANTIC_NODES) {
        let reference = opaque_id("ref");
        session.references.insert(
            reference.clone(),
            ElementTarget {
                path: node.path,
                marker: node.marker,
                document_marker: raw.document_marker.clone(),
                role: node.role.clone(),
                name: node.name.clone(),
                value_class: node.value_class.clone(),
            },
        );
        nodes.push(SemanticNode {
            role: node.role,
            name: node.name,
            value_class: node.value_class,
            visible: node.visible,
            enabled: node.enabled,
            reference,
        });
    }
    let combined = nodes
        .iter()
        .map(|node| format!("{} {}", node.role, node.name))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let possible_prompt_injection = [
        "ignore previous",
        "ignore all previous",
        "system prompt",
        "assistant instructions",
        "developer message",
        "reveal your prompt",
    ]
    .iter()
    .any(|needle| combined.contains(needle));
    let protected_interruption = protected_interruption(&combined);
    Ok(BrowserSnapshot {
        document_generation: session.document_generation,
        url: raw.url.chars().take(2_048).collect(),
        title: raw.title.chars().take(240).collect(),
        captured_at_ms: crate::foundation::clock::unix_time_ms_i64(),
        nodes,
        possible_prompt_injection,
        protected_interruption,
    })
}

fn protected_interruption(text: &str) -> Option<String> {
    for (kind, needles) in [
        (
            "captcha",
            &["captcha", "verify you are human", "not a robot"][..],
        ),
        (
            "mfa",
            &["two-factor", "2fa", "verification code", "one-time code"],
        ),
        (
            "payment",
            &["credit card", "payment method", "cvv", "billing address"],
        ),
        ("password", &["password", "passcode"]),
        (
            "destructive",
            &["delete permanently", "cannot be undone", "erase all"],
        ),
    ] {
        if needles.iter().any(|needle| text.contains(needle)) {
            return Some(kind.to_string());
        }
    }
    None
}

pub(super) async fn click(
    app: &tauri::AppHandle,
    session: &BrowserSession,
    target: &ElementTarget,
) -> Result<(), String> {
    execute_target(app, session, target, "element.click(); true").await
}

pub(super) async fn type_text(
    app: &tauri::AppHandle,
    session: &BrowserSession,
    target: &ElementTarget,
    text: &str,
) -> Result<(), String> {
    if text.chars().count() > MAX_ACTION_TEXT_CHARS {
        return Err("Browser text exceeds the bounded action limit.".to_string());
    }
    if target.value_class == "password" {
        return Err("Password entry requires human takeover.".to_string());
    }
    let encoded = serde_json::to_string(text).map_err(|error| error.to_string())?;
    execute_target(app, session, target, &format!(
        "element.focus(); element.value = {encoded}; element.dispatchEvent(new InputEvent('input', {{bubbles:true, inputType:'insertText'}})); element.dispatchEvent(new Event('change', {{bubbles:true}})); true"
    )).await
}

pub(super) async fn select(
    app: &tauri::AppHandle,
    session: &BrowserSession,
    target: &ElementTarget,
    value: &str,
) -> Result<(), String> {
    let encoded = serde_json::to_string(value).map_err(|error| error.to_string())?;
    execute_target(app, session, target, &format!(
        "if (element.tagName !== 'SELECT') throw new Error('reference_not_select'); element.value = {encoded}; element.dispatchEvent(new Event('change', {{bubbles:true}})); true"
    )).await
}

pub(super) async fn upload(
    app: &tauri::AppHandle,
    session: &BrowserSession,
    target: &ElementTarget,
    file_name: &str,
    mime_type: &str,
    base64_bytes: &str,
) -> Result<(), String> {
    let name = serde_json::to_string(file_name).map_err(|error| error.to_string())?;
    let mime = serde_json::to_string(mime_type).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_string(base64_bytes).map_err(|error| error.to_string())?;
    execute_target(app, session, target, &format!(
        "if (element.tagName !== 'INPUT' || element.type !== 'file') throw new Error('reference_not_file_input'); const raw=atob({bytes}); const data=new Uint8Array(raw.length); for(let i=0;i<raw.length;i++) data[i]=raw.charCodeAt(i); const transfer=new DataTransfer(); transfer.items.add(new File([data], {name}, {{type:{mime}}})); element.files=transfer.files; element.dispatchEvent(new Event('change', {{bubbles:true}})); true"
    )).await
}

pub(super) async fn press_key(app: &tauri::AppHandle, key: &str) -> Result<(), String> {
    let allowed = [
        "Enter",
        "Escape",
        "Tab",
        "ArrowUp",
        "ArrowDown",
        "ArrowLeft",
        "ArrowRight",
        "Backspace",
        "Delete",
        "Space",
    ];
    if !allowed.contains(&key) {
        return Err("Unsupported browser key.".to_string());
    }
    let key = serde_json::to_string(key).map_err(|error| error.to_string())?;
    let script = format!("(() => {{ const target=document.activeElement || document.body; target.dispatchEvent(new KeyboardEvent('keydown', {{key:{key}, bubbles:true}})); target.dispatchEvent(new KeyboardEvent('keyup', {{key:{key}, bubbles:true}})); return true; }})()") ;
    let _: bool = evaluate_json(app, script).await?;
    Ok(())
}

pub(super) async fn scroll(app: &tauri::AppHandle, delta_y: i32) -> Result<(), String> {
    let bounded = delta_y.clamp(-4_000, 4_000);
    let script = format!(
        "(() => {{ window.scrollBy({{top:{bounded}, behavior:'instant'}}); return true; }})()"
    );
    let _: bool = evaluate_json(app, script).await?;
    Ok(())
}

async fn execute_target(
    app: &tauri::AppHandle,
    session: &BrowserSession,
    target: &ElementTarget,
    operation: &str,
) -> Result<(), String> {
    if session.current_document_marker.as_deref() != Some(target.document_marker.as_str()) {
        return Err("Browser reference is stale because the document changed.".to_string());
    }
    let path = serde_json::to_string(&target.path).map_err(|error| error.to_string())?;
    let doc_key =
        serde_json::to_string(&session.document_marker_key).map_err(|error| error.to_string())?;
    let element_key =
        serde_json::to_string(&session.element_marker_key).map_err(|error| error.to_string())?;
    let doc_marker =
        serde_json::to_string(&target.document_marker).map_err(|error| error.to_string())?;
    let marker = serde_json::to_string(&target.marker).map_err(|error| error.to_string())?;
    let role = serde_json::to_string(&target.role).map_err(|error| error.to_string())?;
    let name = serde_json::to_string(&target.name).map_err(|error| error.to_string())?;
    let value_class =
        serde_json::to_string(&target.value_class).map_err(|error| error.to_string())?;
    let script = format!(
        r#"(() => {{
      const docKey={doc_key}, elementKey={element_key};
      if (document[docKey] !== {doc_marker}) throw new Error('stale_document');
      let element=document; for (const index of {path}) {{ element=index===-1?element?.contentDocument:element?.children?.[index]; if(!element) throw new Error('stale_path'); }}
      const role=(element.getAttribute('role') || ({{A:'link',BUTTON:'button',INPUT:element.type==='file'?'file':element.type==='password'?'textbox':'textbox',SELECT:'combobox',TEXTAREA:'textbox'}}[element.tagName] || element.tagName.toLowerCase()));
      const name=((element.getAttribute('aria-label') || element.getAttribute('alt') || element.innerText || element.value || '').replace(/\s+/g,' ').trim()).slice(0,240);
      const valueClass=element.type==='password'?'password':element.type==='file'?'file':element.value?'present':'empty';
      if (element[elementKey] !== {marker} || role !== {role} || name !== {name} || valueClass !== {value_class}) throw new Error('stale_reference');
      return {operation};
    }})()"#
    );
    let result: bool = evaluate_json(app, script).await.map_err(|error| {
        if error.contains("stale_") {
            "Browser reference is stale; take a fresh snapshot.".to_string()
        } else {
            error
        }
    })?;
    if !result {
        return Err("Browser action did not execute.".to_string());
    }
    Ok(())
}

fn snapshot_script(
    document_key: &str,
    element_key: &str,
    new_document_marker: &str,
) -> Result<String, String> {
    let doc_key = serde_json::to_string(document_key).map_err(|error| error.to_string())?;
    let element_key = serde_json::to_string(element_key).map_err(|error| error.to_string())?;
    let document_marker =
        serde_json::to_string(new_document_marker).map_err(|error| error.to_string())?;
    Ok(format!(
        r#"(() => {{
      const docKey={doc_key}, elementKey={element_key};
      if (!Object.prototype.hasOwnProperty.call(document, docKey)) Object.defineProperty(document, docKey, {{value:{document_marker}, configurable:false, enumerable:false}});
      const candidates=[];
      const selector='a,button,input,select,textarea,iframe,[role],[contenteditable="true"],h1,h2,h3,h4,p,li,table';
      const pathOf=(node,root) => {{ const path=[]; while(node && node!==root) {{ const parent=node.parentElement || root; path.unshift(Array.prototype.indexOf.call(parent.children,node)); node=parent; }} return path; }};
      const collect=(root,prefix,depth) => {{ if(!root || depth>3 || candidates.length>={MAX_SEMANTIC_NODES}) return; for(const element of root.querySelectorAll(selector)) {{ if(candidates.length>={MAX_SEMANTIC_NODES}) break; candidates.push({{element,path:prefix.concat(pathOf(element,root))}}); if(element.tagName==='IFRAME') {{ try {{ collect(element.contentDocument,prefix.concat(pathOf(element,root),[-1]),depth+1); }} catch (_) {{}} }} }} }};
      collect(document,[],0);
      const nodes=[];
      for (const candidate of candidates) {{
        const element=candidate.element;
        const style=(element.ownerDocument.defaultView || window).getComputedStyle(element), rect=element.getBoundingClientRect();
        const visible=style.display!=='none' && style.visibility!=='hidden' && rect.width>0 && rect.height>0;
        if(!visible || element.closest('[aria-hidden="true"]')) continue;
        const role=(element.getAttribute('role') || ({{A:'link',BUTTON:'button',INPUT:element.type==='file'?'file':element.type==='password'?'textbox':'textbox',SELECT:'combobox',TEXTAREA:'textbox',IFRAME:'iframe',H1:'heading',H2:'heading',H3:'heading',H4:'heading',P:'paragraph',LI:'listitem',TABLE:'table'}}[element.tagName] || element.tagName.toLowerCase()));
        let name=(element.getAttribute('aria-label') || element.getAttribute('alt') || element.innerText || (element.type==='password'?'':element.value) || '').replace(/\s+/g,' ').trim().slice(0,240);
        const valueClass=element.type==='password'?'password':element.type==='file'?'file':element.value?'present':'empty';
        if (!Object.prototype.hasOwnProperty.call(element, elementKey)) Object.defineProperty(element, elementKey, {{value:crypto.randomUUID(), configurable:false, enumerable:false}});
        nodes.push({{role,name,valueClass,visible:true,enabled:!element.disabled && element.getAttribute('aria-disabled')!=='true',path:candidate.path,marker:element[elementKey]}});
      }}
      return {{documentMarker:document[docKey],url:location.href,title:document.title,nodes}};
    }})()"#
    ))
}

async fn evaluate_json<T: serde::de::DeserializeOwned>(
    app: &tauri::AppHandle,
    script: String,
) -> Result<T, String> {
    let webview = app
        .get_webview(BROWSER_WEBVIEW_LABEL)
        .ok_or_else(|| "The controlled browser view is not open.".to_string())?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = std::sync::Mutex::new(Some(sender));
    webview
        .eval_with_callback(script, move |value| {
            if let Ok(mut guard) = sender.lock() {
                if let Some(sender) = guard.take() {
                    let _ = sender.send(value);
                }
            }
        })
        .map_err(|error| {
            format!("Browser driver could not evaluate a constrained action: {error}")
        })?;
    let encoded = tokio::time::timeout(DRIVER_TIMEOUT, receiver)
        .await
        .map_err(|_| "Browser driver action timed out.".to_string())?
        .map_err(|_| "Browser driver callback was cancelled.".to_string())?;
    let value: serde_json::Value = serde_json::from_str(&encoded)
        .map_err(|error| format!("Browser driver returned invalid JSON: {error}"))?;
    let value = match value {
        serde_json::Value::String(inner) => {
            serde_json::from_str(&inner).unwrap_or(serde_json::Value::String(inner))
        }
        other => other,
    };
    if let Some(error) = value.get("error").and_then(|value| value.as_str()) {
        return Err(format!("Browser driver rejected the action: {error}"));
    }
    serde_json::from_value(value)
        .map_err(|error| format!("Browser driver result was invalid: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_driver_has_no_model_javascript_or_selector_slot() {
        let script = snapshot_script("doc-key", "element-key", "marker").unwrap();
        assert!(script.contains("const selector='a,button,input"));
        assert!(!script.contains("eval("));
        assert!(!script.contains("window.__TAURI"));
        assert!(script.contains("configurable:false"));
        assert!(script.contains("contentDocument"));
        assert!(script.contains("[-1]"));
    }

    #[test]
    fn protected_interruptions_fail_closed() {
        assert_eq!(
            protected_interruption("enter your password"),
            Some("password".to_string())
        );
        assert_eq!(protected_interruption("normal article"), None);
    }
}
