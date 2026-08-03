use super::super::{evidence::CanonicalEvidence, MarginAssessment, RateReconciliation};
use crate::artifacts::presentations::*;

pub(super) const CONTENT_X: i64 = 520_000;
pub(super) const CONTENT_WIDTH: i64 = 11_152_000;

pub(super) fn content_slide(
    slide_id: &str,
    title: &str,
    mut elements: Vec<PresentationElement>,
    speaker_notes: &str,
    source_refs: Vec<String>,
) -> PresentationSlide {
    elements.insert(
        0,
        text_element(
            "slide-title",
            Frame {
                x: CONTENT_X,
                y: 200_000,
                width: CONTENT_WIDTH,
                height: 950_000,
            },
            title,
            35.0,
            true,
            "17365D",
            Vec::new(),
        ),
    );
    PresentationSlide {
        slide_id: slide_id.to_string(),
        layout_id: "decision-pack-layout".to_string(),
        title: Some(title.to_string()),
        elements,
        notes: SlideNotes {
            speaker_notes: speaker_notes.to_string(),
            source_refs,
        },
        animations: Vec::new(),
    }
}

pub(super) fn theme() -> PresentationTheme {
    PresentationTheme {
        theme_id: "decision-pack-theme".to_string(),
        name: "OOMU executive clarity".to_string(),
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
    }
}

pub(super) fn rate_header() -> Vec<TextBlock> {
    table_row(
        &[
            "Supplier / item",
            "Historical",
            "Active quote",
            "Variance",
            "Status",
        ],
        true,
    )
}

pub(super) fn rate_row(rate: &RateReconciliation) -> Vec<TextBlock> {
    table_row(
        &[
            rate.name.clone(),
            format!("{:.2}", rate.historical_rate),
            format!("{:.2}", rate.active_quote),
            format!("{:+.2}", rate.active_quote - rate.historical_rate),
            display_status(&rate.status),
        ],
        false,
    )
}

pub(super) fn margin_header() -> Vec<TextBlock> {
    table_row(
        &[
            "Supplier",
            "Raw cost",
            "COGS",
            "Reported / calculated",
            "Threshold",
            "Decision",
        ],
        true,
    )
}

pub(super) fn margin_row(margin: &MarginAssessment) -> Vec<TextBlock> {
    table_row(
        &[
            margin.name.clone(),
            format!("${:.2}", margin.raw_estimated_cost),
            format!("${:.2}", margin.cogs_allocation),
            format!(
                "{:.2}% / {:.2}%",
                margin.margin_percent,
                ((margin.raw_estimated_cost - margin.cogs_allocation) / margin.raw_estimated_cost)
                    * 100.0
            ),
            format!("{:.2}%", margin.threshold_percent),
            if margin.margin_percent >= margin.threshold_percent {
                "Meets".to_string()
            } else {
                "Below".to_string()
            },
        ],
        false,
    )
}

fn table_row<T: AsRef<str>>(values: &[T], header: bool) -> Vec<TextBlock> {
    values
        .iter()
        .map(|value| {
            text_block(
                value.as_ref(),
                16.0,
                header,
                if header { "FFFFFF" } else { "202124" },
            )
        })
        .collect()
}

pub(super) fn text_element(
    object_id: &str,
    frame: Frame,
    value: &str,
    size: f32,
    bold: bool,
    color: &str,
    provenance: Vec<ProvenanceAnchor>,
) -> PresentationElement {
    PresentationElement {
        object_id: object_id.to_string(),
        frame,
        content: ElementContent::TextBox {
            text: text_block(value, size, bold, color),
        },
        provenance,
    }
}

pub(super) fn table_element(
    object_id: &str,
    frame: Frame,
    table: PresentationTable,
    evidence: &[CanonicalEvidence],
) -> PresentationElement {
    PresentationElement {
        object_id: object_id.to_string(),
        frame,
        content: ElementContent::Table { table },
        provenance: evidence.iter().map(anchor).collect(),
    }
}

pub(super) fn chart_element(
    object_id: &str,
    frame: Frame,
    chart: PresentationChart,
    evidence: &[CanonicalEvidence],
) -> PresentationElement {
    PresentationElement {
        object_id: object_id.to_string(),
        frame,
        content: ElementContent::Chart { chart },
        provenance: evidence.iter().map(anchor).collect(),
    }
}

pub(super) fn callout_element(
    object_id: &str,
    value: &str,
    fill_color: &str,
    text_color: &str,
    provenance: Vec<ProvenanceAnchor>,
) -> PresentationElement {
    PresentationElement {
        object_id: object_id.to_string(),
        frame: Frame {
            x: CONTENT_X,
            y: 1_400_000,
            width: CONTENT_WIDTH,
            height: 4_750_000,
        },
        content: ElementContent::Shape {
            geometry: ShapeGeometry::RoundedRectangle,
            fill_color: fill_color.to_string(),
            line_color: None,
            text: Some(text_block(value, 18.0, false, text_color)),
        },
        provenance,
    }
}

pub(super) fn shape(
    object_id: &str,
    frame: Frame,
    fill_color: &str,
    text: Option<TextBlock>,
) -> PresentationElement {
    PresentationElement {
        object_id: object_id.to_string(),
        frame,
        content: ElementContent::Shape {
            geometry: ShapeGeometry::Rectangle,
            fill_color: fill_color.to_string(),
            line_color: None,
            text,
        },
        provenance: Vec::new(),
    }
}

pub(super) fn text_block(value: &str, size: f32, bold: bool, color: &str) -> TextBlock {
    TextBlock {
        paragraphs: value
            .split('\n')
            .map(|line| TextParagraph {
                runs: vec![TextRun {
                    text: line.to_string(),
                    font_family: "Arial".to_string(),
                    font_size_pt: size,
                    bold,
                    italic: false,
                    color: color.to_string(),
                }],
                alignment: TextAlignment::Left,
                bullet: false,
            })
            .collect(),
        vertical_alignment: VerticalAlignment::Middle,
    }
}

pub(super) fn anchor(evidence: &CanonicalEvidence) -> ProvenanceAnchor {
    ProvenanceAnchor {
        source_ref: evidence.source_ref.clone(),
        evidence_ref: evidence.evidence_ref.clone(),
        note: Some("Canonical decision-pack analysis input".to_string()),
    }
}

fn display_status(status: &str) -> String {
    match status.trim() {
        "PENDING_RECONCILIATION" => "Requires reconciliation".to_string(),
        "ALIGNED" => "Aligned".to_string(),
        value if value.contains('_') => value
            .split('_')
            .filter(|part| !part.is_empty())
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>()
            .join(" "),
        value => value.to_string(),
    }
}
