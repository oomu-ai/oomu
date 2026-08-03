use super::*;

pub fn deterministic_presentation_fixture() -> PresentationIr {
    let title = text_block("Quarterly operating review", 28.0, true);
    let body = text_block(
        "Revenue increased while support response time improved.",
        18.0,
        false,
    );
    let table = PresentationTable {
        header_row: true,
        rows: vec![
            vec![
                text_block("Metric", 12.0, true),
                text_block("Result", 12.0, true),
            ],
            vec![
                text_block("Revenue", 12.0, false),
                text_block("+12%", 12.0, false),
            ],
        ],
    };
    PresentationIr {
        schema_version: PRESENTATION_IR_VERSION,
        title: "Quarterly operating review".to_string(),
        locale: "en-US".to_string(),
        revision: 1,
        aspect_ratio: PresentationAspectRatio::Widescreen,
        theme: PresentationTheme {
            theme_id: "theme-main".to_string(),
            name: "OOMU clarity".to_string(),
            colors: ThemeColors {
                dark: "202124".to_string(),
                light: "FFFFFF".to_string(),
                accent_1: "0B57D0".to_string(),
                accent_2: "188038".to_string(),
                accent_3: "B06000".to_string(),
                accent_4: "A50E0E".to_string(),
                hyperlink: "0B57D0".to_string(),
            },
            fonts: ThemeFonts {
                heading: "Arial".to_string(),
                body: "Arial".to_string(),
            },
        },
        masters: vec![SlideMaster {
            master_id: "master-main".to_string(),
            name: "Primary master".to_string(),
            theme_id: "theme-main".to_string(),
            layout_ids: vec!["layout-content".to_string()],
        }],
        layouts: vec![SlideLayout {
            layout_id: "layout-content".to_string(),
            master_id: "master-main".to_string(),
            name: "Title and content".to_string(),
            kind: SlideLayoutKind::TitleAndContent,
            placeholders: Vec::new(),
        }],
        slides: vec![PresentationSlide {
            slide_id: "slide-summary".to_string(),
            layout_id: "layout-content".to_string(),
            title: Some("Operating summary".to_string()),
            elements: vec![
                PresentationElement {
                    object_id: "title".to_string(),
                    frame: Frame {
                        x: 600_000,
                        y: 300_000,
                        width: 10_900_000,
                        height: 900_000,
                    },
                    content: ElementContent::TextBox { text: title },
                    provenance: Vec::new(),
                },
                PresentationElement {
                    object_id: "summary".to_string(),
                    frame: Frame {
                        x: 600_000,
                        y: 1_400_000,
                        width: 5_200_000,
                        height: 1_200_000,
                    },
                    content: ElementContent::TextBox { text: body },
                    provenance: Vec::new(),
                },
                PresentationElement {
                    object_id: "metrics-table".to_string(),
                    frame: Frame {
                        x: 600_000,
                        y: 2_900_000,
                        width: 5_200_000,
                        height: 2_400_000,
                    },
                    content: ElementContent::Table { table },
                    provenance: Vec::new(),
                },
                PresentationElement {
                    object_id: "trend-chart".to_string(),
                    frame: Frame {
                        x: 6_200_000,
                        y: 1_400_000,
                        width: 5_300_000,
                        height: 3_900_000,
                    },
                    content: ElementContent::Chart {
                        chart: PresentationChart {
                            chart_type: ChartType::Column,
                            title: "Revenue index".to_string(),
                            categories: vec!["Q1".to_string(), "Q2".to_string()],
                            series: vec![ChartSeries {
                                name: "Revenue".to_string(),
                                values: vec![100.0, 112.0],
                            }],
                        },
                    },
                    provenance: Vec::new(),
                },
            ],
            notes: SlideNotes {
                speaker_notes: "Confirm final values before presenting.".to_string(),
                source_refs: Vec::new(),
            },
            animations: Vec::new(),
        }],
        citations: Vec::new(),
        policy: PresentationPolicy::default(),
        template: PresentationTemplateIdentity::default(),
    }
}

fn text_block(value: &str, size: f32, bold: bool) -> TextBlock {
    TextBlock {
        paragraphs: vec![TextParagraph {
            runs: vec![TextRun {
                text: value.to_string(),
                font_family: "Arial".to_string(),
                font_size_pt: size,
                bold,
                italic: false,
                color: "202124".to_string(),
            }],
            alignment: TextAlignment::Left,
            bullet: false,
        }],
        vertical_alignment: VerticalAlignment::Top,
    }
}
