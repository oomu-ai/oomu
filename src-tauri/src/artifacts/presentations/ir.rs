use serde::{Deserialize, Serialize};

pub const PRESENTATION_IR_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationIr {
    pub schema_version: u16,
    pub title: String,
    pub locale: String,
    pub revision: u32,
    pub aspect_ratio: PresentationAspectRatio,
    pub theme: PresentationTheme,
    pub masters: Vec<SlideMaster>,
    pub layouts: Vec<SlideLayout>,
    pub slides: Vec<PresentationSlide>,
    #[serde(default)]
    pub citations: Vec<PresentationCitation>,
    #[serde(default)]
    pub policy: PresentationPolicy,
    #[serde(default)]
    pub template: PresentationTemplateIdentity,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PresentationAspectRatio {
    #[serde(rename = "16:9")]
    Widescreen,
    #[serde(rename = "4:3")]
    Standard,
}

impl PresentationAspectRatio {
    pub fn dimensions_emu(self) -> (i64, i64) {
        match self {
            Self::Widescreen => (12_192_000, 6_858_000),
            Self::Standard => (9_144_000, 6_858_000),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationTheme {
    pub theme_id: String,
    pub name: String,
    pub colors: ThemeColors,
    pub fonts: ThemeFonts,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeColors {
    pub dark: String,
    pub light: String,
    pub accent_1: String,
    pub accent_2: String,
    pub accent_3: String,
    pub accent_4: String,
    pub hyperlink: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeFonts {
    pub heading: String,
    pub body: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlideMaster {
    pub master_id: String,
    pub name: String,
    pub theme_id: String,
    pub layout_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlideLayout {
    pub layout_id: String,
    pub master_id: String,
    pub name: String,
    pub kind: SlideLayoutKind,
    #[serde(default)]
    pub placeholders: Vec<SlidePlaceholder>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlideLayoutKind {
    Title,
    TitleAndContent,
    SectionHeader,
    TwoColumn,
    Blank,
    Custom,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlidePlaceholder {
    pub placeholder_id: String,
    pub kind: PlaceholderKind,
    pub frame: Frame,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceholderKind {
    Title,
    Subtitle,
    Body,
    Picture,
    Chart,
    Table,
    Footer,
    SlideNumber,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationSlide {
    pub slide_id: String,
    pub layout_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub elements: Vec<PresentationElement>,
    #[serde(default)]
    pub notes: SlideNotes,
    #[serde(default)]
    pub animations: Vec<SlideAnimation>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlideNotes {
    #[serde(default)]
    pub speaker_notes: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SlideAnimation {
    pub animation_id: String,
    pub object_id: String,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationElement {
    pub object_id: String,
    pub frame: Frame,
    pub content: ElementContent,
    #[serde(default)]
    pub provenance: Vec<ProvenanceAnchor>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ElementContent {
    TextBox {
        text: TextBlock,
    },
    Shape {
        geometry: ShapeGeometry,
        fill_color: String,
        #[serde(default)]
        line_color: Option<String>,
        #[serde(default)]
        text: Option<TextBlock>,
    },
    Image {
        image: PresentationImage,
    },
    Table {
        table: PresentationTable,
    },
    Chart {
        chart: PresentationChart,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeGeometry {
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Triangle,
    Line,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Frame {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextBlock {
    #[serde(default)]
    pub paragraphs: Vec<TextParagraph>,
    #[serde(default)]
    pub vertical_alignment: VerticalAlignment,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextParagraph {
    #[serde(default)]
    pub runs: Vec<TextRun>,
    #[serde(default)]
    pub alignment: TextAlignment,
    #[serde(default)]
    pub bullet: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextRun {
    pub text: String,
    pub font_family: String,
    pub font_size_pt: f32,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default = "default_text_color")]
    pub color: String,
}

fn default_text_color() -> String {
    "202124".to_string()
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalAlignment {
    #[default]
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationImage {
    pub asset_id: String,
    pub media_type: ImageMediaType,
    pub bytes_base64: String,
    pub width_px: u32,
    pub height_px: u32,
    pub alt_text: String,
    pub license: ImageLicense,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMediaType {
    Png,
    Jpeg,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageLicense {
    pub status: ImageLicenseStatus,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub attribution: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageLicenseStatus {
    Owned,
    Licensed,
    PublicDomain,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationTable {
    pub rows: Vec<Vec<TextBlock>>,
    #[serde(default)]
    pub header_row: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationChart {
    pub chart_type: ChartType,
    pub title: String,
    pub categories: Vec<String>,
    pub series: Vec<ChartSeries>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartType {
    Column,
    Bar,
    Line,
    Pie,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartSeries {
    pub name: String,
    pub values: Vec<f64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProvenanceAnchor {
    pub source_ref: String,
    pub evidence_ref: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationCitation {
    pub citation_id: String,
    pub slide_id: String,
    #[serde(default)]
    pub object_id: Option<String>,
    pub source_ref: String,
    pub evidence_ref: String,
    pub label: String,
    #[serde(default)]
    pub locator: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationPolicy {
    pub overflow: OverflowPolicy,
    pub missing_font: MissingFontPolicy,
    pub image_license: ImageLicensePolicy,
    pub unsupported_animation: UnsupportedAnimationPolicy,
    pub minimum_font_size_pt: f32,
    pub minimum_image_dpi: u32,
    #[serde(default)]
    pub allowed_fonts: Vec<String>,
}

impl Default for PresentationPolicy {
    fn default() -> Self {
        Self {
            overflow: OverflowPolicy::Reject,
            missing_font: MissingFontPolicy::Reject,
            image_license: ImageLicensePolicy::RequireKnown,
            unsupported_animation: UnsupportedAnimationPolicy::Reject,
            minimum_font_size_pt: 10.0,
            minimum_image_dpi: 144,
            allowed_fonts: vec!["Arial".to_string(), "Georgia".to_string()],
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverflowPolicy {
    Reject,
    ShrinkToFit,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingFontPolicy {
    Reject,
    SubstituteTheme,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageLicensePolicy {
    RequireKnown,
    AllowUnknownWithWarning,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedAnimationPolicy {
    Reject,
    Remove,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresentationTemplateIdentity {
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default = "default_template_name")]
    pub name: String,
    #[serde(default)]
    pub imported: bool,
    #[serde(default)]
    pub fingerprint_sha256: String,
}

fn default_template_name() -> String {
    "OOMU native".to_string()
}

impl Default for PresentationTemplateIdentity {
    fn default() -> Self {
        Self {
            template_id: None,
            name: default_template_name(),
            imported: false,
            fingerprint_sha256: String::new(),
        }
    }
}
