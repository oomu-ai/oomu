pub(super) fn requested_file_formats(lowered: &str) -> Vec<&'static str> {
    [
        ("docx", &[".docx", "word document"][..]),
        ("pdf", &[".pdf", "pdf document"][..]),
        ("md", &[".markdown", ".md", "markdown file"][..]),
        ("txt", &[".txt", "text file"][..]),
        ("rtf", &[".rtf", "rich text"][..]),
        ("csv", &[".csv", "csv file"][..]),
        (
            "xlsx",
            &[".xlsx", "excel workbook", "excel spreadsheet"][..],
        ),
        ("xls", &[".xls", "excel 97 workbook"][..]),
        (
            "pptx",
            &[".pptx", "powerpoint presentation", "powerpoint file"][..],
        ),
        ("json", &[".json", "json file"][..]),
        ("html", &[".html", "html file"][..]),
        ("xml", &[".xml", "xml file"][..]),
    ]
    .into_iter()
    .filter_map(|(format, needles)| {
        needles
            .iter()
            .any(|needle| {
                if *needle == ".xls" && lowered.contains(".xlsx") {
                    lowered.replace(".xlsx", "").contains(".xls")
                } else {
                    lowered.contains(needle)
                }
            })
            .then_some(format)
    })
    .collect()
}
