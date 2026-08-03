use super::{xml, PresentationAspectRatio, PresentationIr};
use sha2::{Digest, Sha256};

const XML_HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;

pub(crate) fn core_properties(presentation: &PresentationIr) -> String {
    format!(
        r#"{XML_HEAD}<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/"><dc:title>{}</dc:title><dc:creator>OOMU</dc:creator><cp:revision>{}</cp:revision></cp:coreProperties>"#,
        xml::text(&presentation.title),
        presentation.revision
    )
}

pub(crate) fn app_properties(presentation: &PresentationIr) -> String {
    format!(
        r#"{XML_HEAD}<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>OOMU</Application><Slides>{}</Slides><Notes>{}</Notes><PresentationFormat>{}</PresentationFormat></Properties>"#,
        presentation.slides.len(),
        presentation.slides.len(),
        match presentation.aspect_ratio {
            PresentationAspectRatio::Widescreen => "Widescreen",
            PresentationAspectRatio::Standard => "Standard",
        }
    )
}

pub(crate) fn embedded_ir_xml(presentation: &PresentationIr) -> Result<String, String> {
    let json = serde_json::to_string(presentation).map_err(|error| error.to_string())?;
    Ok(format!(
        r#"{XML_HEAD}<oomu:presentationContract xmlns:oomu="https://oomu.local/contracts/presentation/v1"><oomu:json>{}</oomu:json></oomu:presentationContract>"#,
        xml::text(&json)
    ))
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
