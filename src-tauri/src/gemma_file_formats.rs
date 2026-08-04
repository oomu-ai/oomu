pub(super) fn requested_file_formats(lowered: &str) -> Vec<&'static str> {
    [
        (
            "docx",
            &[
                ".docx",
                "docx",
                "word document",
                "word doc",
                "word file",
                "microsoft word",
            ][..],
        ),
        ("pdf", &[".pdf", "pdf", "pdf document", "pdf file"][..]),
        ("md", &[".markdown", ".md", "markdown", "markdown file"][..]),
        ("txt", &[".txt", "txt", "text file", "plain text"][..]),
        ("rtf", &[".rtf", "rtf", "rich text"][..]),
        ("csv", &[".csv", "csv", "csv file"][..]),
        (
            "xlsx",
            &[
                ".xlsx",
                "xlsx",
                "excel workbook",
                "excel spreadsheet",
                "excel document",
                "excel file",
                "microsoft excel",
                "spreadsheet",
            ][..],
        ),
        ("xls", &[".xls", "xls", "excel 97 workbook"][..]),
        (
            "pptx",
            &[
                ".pptx",
                "pptx",
                "powerpoint",
                "powerpoint presentation",
                "powerpoint file",
                "powerpoint deck",
                "slide deck",
                "presentation deck",
            ][..],
        ),
        ("json", &[".json", "json", "json file"][..]),
        ("html", &[".html", "html", "html file"][..]),
        ("xml", &[".xml", "xml", "xml file"][..]),
    ]
    .into_iter()
    .filter_map(|(format, needles)| {
        needles
            .iter()
            .any(|needle| contains_alias(lowered, needle))
            .then_some(format)
    })
    .collect()
}

fn contains_alias(value: &str, alias: &str) -> bool {
    value.match_indices(alias).any(|(index, _)| {
        let after = value[index + alias.len()..].chars().next();
        if alias.starts_with('.') {
            return after.is_none_or(|character| !character.is_ascii_alphanumeric());
        }
        let before = value[..index].chars().next_back();
        before.is_none_or(|character| !character.is_ascii_alphanumeric())
            && after.is_none_or(|character| !character.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use super::requested_file_formats;

    #[test]
    fn common_product_names_map_to_the_registered_formats() {
        for (request, expected) in [
            ("create a Word doc", "docx"),
            ("create a PDF", "pdf"),
            ("create a PowerPoint", "pptx"),
            ("create an Excel file", "xlsx"),
            ("create a slide deck", "pptx"),
            ("create a spreadsheet", "xlsx"),
        ] {
            assert_eq!(
                requested_file_formats(&request.to_ascii_lowercase()),
                vec![expected],
                "{request}"
            );
        }
    }

    #[test]
    fn legacy_extensions_do_not_collide_with_modern_extensions() {
        assert_eq!(requested_file_formats("create report.xlsx"), vec!["xlsx"]);
        assert_eq!(requested_file_formats("create report.xls"), vec!["xls"]);
        assert_eq!(requested_file_formats("create deck.pptx"), vec!["pptx"]);
    }
}
