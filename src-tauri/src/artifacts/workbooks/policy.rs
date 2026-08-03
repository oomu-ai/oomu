use std::collections::BTreeMap;

const FORBIDDEN_PATH_FRAGMENTS: [&str; 14] = [
    "vbaproject",
    "activex",
    "embeddings/",
    "oleobject",
    "externallinks/",
    "connections.xml",
    "querytables/",
    "pivotcache/",
    "model/",
    "customui/",
    "ctrlprops/",
    "webextensions/",
    "persons/",
    "threadedcomments/",
];

const FORBIDDEN_RELATIONSHIP_FRAGMENTS: [&str; 11] = [
    "/vbaproject",
    "/activex",
    "/oleobject",
    "/package",
    "/externallink",
    "/connections",
    "/querytable",
    "/webextension",
    "/control",
    "/attachedtemplate",
    "/hyperlink",
];

pub(crate) fn enforce_safe_package(entries: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    for (name, bytes) in entries {
        let lowercase = name.to_ascii_lowercase();
        if FORBIDDEN_PATH_FRAGMENTS
            .iter()
            .any(|fragment| lowercase.contains(fragment))
        {
            return Err(format!(
                "Workbook package contains forbidden active or external part {name}."
            ));
        }
        if is_xml_part(name) {
            let xml = std::str::from_utf8(bytes)
                .map_err(|_| format!("Workbook XML part {name} is not UTF-8."))?;
            let uppercase = xml.to_ascii_uppercase();
            if uppercase.contains("<!DOCTYPE")
                || uppercase.contains("<!ENTITY")
                || uppercase.contains("<MC:ALTERNATECONTENT")
            {
                return Err(format!(
                    "Workbook XML part {name} contains forbidden executable or entity content."
                ));
            }
            if name.ends_with(".rels") {
                inspect_relationships(name, xml)?;
            }
        }
    }
    let content_types = entries
        .get("[Content_Types].xml")
        .ok_or_else(|| "Workbook content-types part is missing.".to_string())?;
    let types = String::from_utf8_lossy(content_types).to_ascii_lowercase();
    if [
        "macroenabled",
        "vnd.ms-office.vba",
        "activex",
        "oleobject",
        "externalLink",
        "connections",
    ]
    .iter()
    .any(|value| types.contains(&value.to_ascii_lowercase()))
    {
        return Err("Workbook content-types declare active or external content.".to_string());
    }
    Ok(())
}

fn inspect_relationships(name: &str, xml: &str) -> Result<(), String> {
    let relationship =
        regex::Regex::new(r"<Relationship\b[^>]*/?>").map_err(|error| error.to_string())?;
    for tag in relationship.find_iter(xml) {
        let lowercase = tag.as_str().to_ascii_lowercase();
        if lowercase.contains("targetmode=\"external\"")
            || lowercase.contains("targetmode='external'")
        {
            return Err(format!(
                "Workbook relationship part {name} contains an external target."
            ));
        }
        let relation_type = attribute(tag.as_str(), "Type")
            .unwrap_or_default()
            .to_ascii_lowercase();
        for fragment in FORBIDDEN_RELATIONSHIP_FRAGMENTS {
            if relation_type.ends_with(fragment) {
                return Err(format!(
                    "Workbook relationship part {name} contains a forbidden relationship type."
                ));
            }
        }
    }
    Ok(())
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let pattern = regex::Regex::new(&format!(
        r#"\b{}\s*=\s*[\"']([^\"']*)[\"']"#,
        regex::escape(name)
    ))
    .ok()?;
    pattern
        .captures(tag)
        .and_then(|captures| captures.get(1).map(|value| value.as_str().to_string()))
}

fn is_xml_part(name: &str) -> bool {
    name.ends_with(".xml") || name.ends_with(".rels") || name.ends_with(".vml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_macros_and_external_relationships() {
        let base = BTreeMap::from([("[Content_Types].xml".to_string(), b"<Types/>".to_vec())]);
        assert!(enforce_safe_package(&base).is_ok());
        let mut macro_package = base.clone();
        macro_package.insert("xl/vbaProject.bin".to_string(), vec![1]);
        assert!(enforce_safe_package(&macro_package).is_err());
        let mut external = base;
        external.insert(
            "xl/_rels/workbook.xml.rels".to_string(),
            b"<Relationships><Relationship TargetMode=\"External\"/></Relationships>".to_vec(),
        );
        assert!(enforce_safe_package(&external).is_err());
    }
}
