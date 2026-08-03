use super::{
    layout_xml::{layout_xml, master_relationships, master_xml, theme_xml},
    package_metadata::{app_properties, core_properties, embedded_ir_xml},
    xml,
    zip::write_store_zip,
    *,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::collections::{BTreeMap, HashMap};

mod table;
use table::table_shape;

pub(crate) use super::package_metadata::hex_digest;

const XML_HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const DOC_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

#[derive(Clone, Debug, PartialEq)]
pub struct BuiltPresentation {
    pub bytes: Vec<u8>,
    pub normalized: PresentationIr,
    pub policy_notices: Vec<PolicyNotice>,
    pub package_sha256: String,
}

pub fn build_presentation(input: &PresentationIr) -> Result<BuiltPresentation, String> {
    let policy = apply_presentation_policies(input)?;
    let entries = package_entries(&policy.presentation)?;
    let bytes = write_store_zip(&entries)?;
    let package_sha256 = hex_digest(&bytes);
    Ok(BuiltPresentation {
        bytes,
        normalized: policy.presentation,
        policy_notices: policy.notices,
        package_sha256,
    })
}

pub(crate) fn package_entries(
    presentation: &PresentationIr,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut entries = BTreeMap::new();
    let layout_indexes = presentation
        .layouts
        .iter()
        .enumerate()
        .map(|(index, layout)| (layout.layout_id.as_str(), index + 1))
        .collect::<HashMap<_, _>>();
    let master_indexes = presentation
        .masters
        .iter()
        .enumerate()
        .map(|(index, master)| (master.master_id.as_str(), index + 1))
        .collect::<HashMap<_, _>>();
    let mut media_index = 0_usize;
    let mut chart_index = 0_usize;
    let mut slide_resources = Vec::new();
    for slide in &presentation.slides {
        let mut resources = SlideResources::default();
        for element in &slide.elements {
            match &element.content {
                ElementContent::Image { image } => {
                    media_index += 1;
                    let extension = match image.media_type {
                        ImageMediaType::Png => "png",
                        ImageMediaType::Jpeg => "jpg",
                    };
                    let path = format!("ppt/media/image{media_index}.{extension}");
                    entries.insert(
                        path.clone(),
                        STANDARD.decode(&image.bytes_base64).map_err(|_| {
                            format!("Image {} has invalid base64 content.", image.asset_id)
                        })?,
                    );
                    resources.images.insert(element.object_id.clone(), path);
                    resources.relations.push(ResourceRelationship {
                        object_id: element.object_id.clone(),
                        target: resources.images[&element.object_id].clone(),
                        kind: "image",
                    });
                }
                ElementContent::Chart { chart } => {
                    chart_index += 1;
                    let path = format!("ppt/charts/chart{chart_index}.xml");
                    entries.insert(path.clone(), chart_xml(chart, chart_index)?.into_bytes());
                    resources.charts.insert(element.object_id.clone(), path);
                    resources.relations.push(ResourceRelationship {
                        object_id: element.object_id.clone(),
                        target: resources.charts[&element.object_id].clone(),
                        kind: "chart",
                    });
                }
                _ => {}
            }
        }
        slide_resources.push(resources);
    }
    entries.insert(
        "[Content_Types].xml".to_string(),
        content_types(presentation, chart_index).into_bytes(),
    );
    entries.insert("_rels/.rels".to_string(), root_relationships().into_bytes());
    entries.insert(
        "docProps/core.xml".to_string(),
        core_properties(presentation).into_bytes(),
    );
    entries.insert(
        "docProps/app.xml".to_string(),
        app_properties(presentation).into_bytes(),
    );
    entries.insert(
        "ppt/presentation.xml".to_string(),
        presentation_xml(presentation).into_bytes(),
    );
    entries.insert(
        "ppt/_rels/presentation.xml.rels".to_string(),
        presentation_relationships(presentation).into_bytes(),
    );
    entries.insert(
        "ppt/presProps.xml".to_string(),
        format!(
            r#"{XML_HEAD}<p:presentationPr xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"/>"#
        )
        .into_bytes(),
    );
    entries.insert(
        "ppt/viewProps.xml".to_string(),
        format!(r#"{XML_HEAD}<p:viewPr xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"/>"#).into_bytes(),
    );
    entries.insert(
        "ppt/tableStyles.xml".to_string(),
        format!(r#"{XML_HEAD}<a:tblStyleLst xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" def="{{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}}"/>"#).into_bytes(),
    );
    entries.insert(
        "ppt/theme/theme1.xml".to_string(),
        theme_xml(&presentation.theme)?.into_bytes(),
    );
    entries.insert(
        "ppt/notesMasters/notesMaster1.xml".to_string(),
        notes_master_xml().into_bytes(),
    );
    entries.insert(
        "ppt/notesMasters/_rels/notesMaster1.xml.rels".to_string(),
        relationships(&[("rId1", "../theme/theme1.xml", "theme")]).into_bytes(),
    );
    entries.insert(
        "customXml/item1.xml".to_string(),
        embedded_ir_xml(presentation)?.into_bytes(),
    );
    for (index, master) in presentation.masters.iter().enumerate() {
        let number = index + 1;
        entries.insert(
            format!("ppt/slideMasters/slideMaster{number}.xml"),
            master_xml(master, &layout_indexes)?.into_bytes(),
        );
        entries.insert(
            format!("ppt/slideMasters/_rels/slideMaster{number}.xml.rels"),
            master_relationships(master, &layout_indexes).into_bytes(),
        );
    }
    for (index, layout) in presentation.layouts.iter().enumerate() {
        let number = index + 1;
        entries.insert(
            format!("ppt/slideLayouts/slideLayout{number}.xml"),
            layout_xml(layout).into_bytes(),
        );
        let master_number = master_indexes[layout.master_id.as_str()];
        entries.insert(
            format!("ppt/slideLayouts/_rels/slideLayout{number}.xml.rels"),
            relationships(&[(
                &"rId1",
                &format!("../slideMasters/slideMaster{master_number}.xml"),
                "slideMaster",
            )])
            .into_bytes(),
        );
    }
    for (index, slide) in presentation.slides.iter().enumerate() {
        let number = index + 1;
        let resources = &slide_resources[index];
        entries.insert(
            format!("ppt/slides/slide{number}.xml"),
            slide_xml(slide, resources)?.into_bytes(),
        );
        let layout_number = layout_indexes[slide.layout_id.as_str()];
        entries.insert(
            format!("ppt/slides/_rels/slide{number}.xml.rels"),
            slide_relationships(number, layout_number, resources).into_bytes(),
        );
        entries.insert(
            format!("ppt/notesSlides/notesSlide{number}.xml"),
            notes_slide_xml(slide, &presentation.citations).into_bytes(),
        );
        entries.insert(
            format!("ppt/notesSlides/_rels/notesSlide{number}.xml.rels"),
            relationships(&[
                ("rId1", &format!("../slides/slide{number}.xml"), "slide"),
                ("rId2", "../notesMasters/notesMaster1.xml", "notesMaster"),
            ])
            .into_bytes(),
        );
    }
    Ok(entries)
}

#[derive(Default)]
struct SlideResources {
    images: BTreeMap<String, String>,
    charts: BTreeMap<String, String>,
    relations: Vec<ResourceRelationship>,
}

struct ResourceRelationship {
    object_id: String,
    target: String,
    kind: &'static str,
}

fn content_types(presentation: &PresentationIr, charts: usize) -> String {
    let mut overrides = String::new();
    for index in 1..=presentation.masters.len() {
        overrides.push_str(&override_part(
            &format!("/ppt/slideMasters/slideMaster{index}.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml",
        ));
    }
    for index in 1..=presentation.layouts.len() {
        overrides.push_str(&override_part(
            &format!("/ppt/slideLayouts/slideLayout{index}.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml",
        ));
    }
    for index in 1..=presentation.slides.len() {
        overrides.push_str(&override_part(
            &format!("/ppt/slides/slide{index}.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        ));
        overrides.push_str(&override_part(
            &format!("/ppt/notesSlides/notesSlide{index}.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml",
        ));
    }
    for index in 1..=charts {
        overrides.push_str(&override_part(
            &format!("/ppt/charts/chart{index}.xml"),
            "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
        ));
    }
    format!(
        r#"{XML_HEAD}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpg" ContentType="image/jpeg"/>{}<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/><Override PartName="/ppt/notesMasters/notesMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml"/><Override PartName="/ppt/presProps.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presProps+xml"/><Override PartName="/ppt/viewProps.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml"/><Override PartName="/ppt/tableStyles.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/></Types>"#,
        overrides
    )
}

fn override_part(name: &str, content_type: &str) -> String {
    format!(r#"<Override PartName="{name}" ContentType="{content_type}"/>"#)
}

fn root_relationships() -> String {
    format!(
        r#"{XML_HEAD}<Relationships xmlns="{REL_NS}"><Relationship Id="rId1" Type="{DOC_REL}/officeDocument" Target="ppt/presentation.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="{DOC_REL}/extended-properties" Target="docProps/app.xml"/></Relationships>"#
    )
}

fn presentation_xml(presentation: &PresentationIr) -> String {
    let masters = (0..presentation.masters.len())
        .map(|index| {
            format!(
                r#"<p:sldMasterId id="{}" r:id="rId{}"/>"#,
                2_147_483_648_u64 + index as u64,
                index + 1
            )
        })
        .collect::<String>();
    let slide_start = presentation.masters.len();
    let slides = (0..presentation.slides.len())
        .map(|index| {
            format!(
                r#"<p:sldId id="{}" r:id="rId{}"/>"#,
                256 + index,
                slide_start + index + 1
            )
        })
        .collect::<String>();
    let notes_master_rid = slide_start + presentation.slides.len() + 1;
    let (width, height) = presentation.aspect_ratio.dimensions_emu();
    let size_type = match presentation.aspect_ratio {
        PresentationAspectRatio::Widescreen => "screen16x9",
        PresentationAspectRatio::Standard => "screen4x3",
    };
    format!(
        r#"{XML_HEAD}<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{DOC_REL}" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldMasterIdLst>{masters}</p:sldMasterIdLst><p:notesMasterIdLst><p:notesMasterId r:id="rId{notes_master_rid}"/></p:notesMasterIdLst><p:sldIdLst>{slides}</p:sldIdLst><p:sldSz cx="{width}" cy="{height}" type="{size_type}"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#
    )
}

fn presentation_relationships(presentation: &PresentationIr) -> String {
    let mut rels = Vec::new();
    for index in 0..presentation.masters.len() {
        rels.push((
            format!("rId{}", index + 1),
            format!("slideMasters/slideMaster{}.xml", index + 1),
            "slideMaster",
        ));
    }
    let start = presentation.masters.len();
    for index in 0..presentation.slides.len() {
        rels.push((
            format!("rId{}", start + index + 1),
            format!("slides/slide{}.xml", index + 1),
            "slide",
        ));
    }
    let next = start + presentation.slides.len() + 1;
    rels.extend([
        (
            format!("rId{next}"),
            "notesMasters/notesMaster1.xml".to_string(),
            "notesMaster",
        ),
        (
            format!("rId{}", next + 1),
            "presProps.xml".to_string(),
            "presProps",
        ),
        (
            format!("rId{}", next + 2),
            "viewProps.xml".to_string(),
            "viewProps",
        ),
        (
            format!("rId{}", next + 3),
            "tableStyles.xml".to_string(),
            "tableStyles",
        ),
    ]);
    let mut xml = format!(r#"{XML_HEAD}<Relationships xmlns="{REL_NS}">"#);
    for (id, target, kind) in rels {
        xml.push_str(&relationship(&id, &target, kind));
    }
    xml.push_str("</Relationships>");
    xml
}

fn slide_xml(slide: &PresentationSlide, resources: &SlideResources) -> Result<String, String> {
    let mut tree = shape_tree_root();
    let insert = tree.len() - "</p:spTree>".len();
    let mut objects = String::new();
    for (index, element) in slide.elements.iter().enumerate() {
        let id = index + 2;
        objects.push_str(&element_xml(id, element, resources)?);
    }
    tree.insert_str(insert, &objects);
    Ok(format!(
        r#"{XML_HEAD}<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{DOC_REL}" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld name="{}">{tree}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#,
        xml::attr(slide.title.as_deref().unwrap_or(&slide.slide_id))
    ))
}

fn element_xml(
    id: usize,
    element: &PresentationElement,
    resources: &SlideResources,
) -> Result<String, String> {
    let relationship_id = || {
        resources
            .relations
            .iter()
            .position(|value| value.object_id == element.object_id)
            .map(|index| format!("rId{}", index + 3))
            .ok_or_else(|| {
                format!(
                    "Resource relationship for {} is missing.",
                    element.object_id
                )
            })
    };
    let value = match &element.content {
        ElementContent::TextBox { text } => text_shape(
            id,
            &element.object_id,
            element.frame,
            "rect",
            None,
            None,
            text,
        )?,
        ElementContent::Shape {
            geometry,
            fill_color,
            line_color,
            text,
        } => text_shape(
            id,
            &element.object_id,
            element.frame,
            geometry_name(*geometry),
            Some(fill_color),
            line_color.as_deref(),
            text.as_ref().unwrap_or(&TextBlock::default()),
        )?,
        ElementContent::Image { image } => {
            let target = resources
                .images
                .get(&element.object_id)
                .ok_or_else(|| "Image resource is missing.".to_string())?;
            let rid = relationship_id()?;
            image_shape(id, element, image, &rid, target)
        }
        ElementContent::Table { table } => table_shape(id, element, table)?,
        ElementContent::Chart { chart } => {
            if !resources.charts.contains_key(&element.object_id) {
                return Err("Chart resource is missing.".to_string());
            }
            let rid = relationship_id()?;
            chart_shape(id, element, chart, &rid)
        }
    };
    Ok(value)
}

fn text_shape(
    id: usize,
    name: &str,
    frame: Frame,
    geometry: &str,
    fill: Option<&str>,
    line: Option<&str>,
    text_block: &TextBlock,
) -> Result<String, String> {
    let fill_xml = match fill {
        Some(value) => format!(
            r#"<a:solidFill><a:srgbClr val="{}"/></a:solidFill>"#,
            xml::color(value)?
        ),
        None => "<a:noFill/>".to_string(),
    };
    let line_xml = match line {
        Some(value) => format!(
            r#"<a:ln><a:solidFill><a:srgbClr val="{}"/></a:solidFill></a:ln>"#,
            xml::color(value)?
        ),
        None => "<a:ln><a:noFill/></a:ln>".to_string(),
    };
    Ok(format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="{geometry}"><a:avLst/></a:prstGeom>{fill_xml}{line_xml}</p:spPr>{}</p:sp>"#,
        xml::attr(name),
        frame.x,
        frame.y,
        frame.width,
        frame.height,
        text_body(text_block)?
    ))
}

fn text_body(block: &TextBlock) -> Result<String, String> {
    let anchor = match block.vertical_alignment {
        VerticalAlignment::Top => "t",
        VerticalAlignment::Middle => "ctr",
        VerticalAlignment::Bottom => "b",
    };
    let paragraphs = if block.paragraphs.is_empty() {
        "<a:p/>".to_string()
    } else {
        block
            .paragraphs
            .iter()
            .map(paragraph_xml)
            .collect::<Result<String, String>>()?
    };
    Ok(format!(
        r#"<p:txBody><a:bodyPr anchor="{anchor}"/><a:lstStyle/>{paragraphs}</p:txBody>"#
    ))
}

fn paragraph_xml(paragraph: &TextParagraph) -> Result<String, String> {
    let alignment = match paragraph.alignment {
        TextAlignment::Left => "l",
        TextAlignment::Center => "ctr",
        TextAlignment::Right => "r",
    };
    let bullet = if paragraph.bullet {
        "<a:buChar char=\"•\"/>"
    } else {
        "<a:buNone/>"
    };
    let runs = paragraph.runs.iter().map(|run| {
        let size = (run.font_size_pt * 100.0).round() as i32;
        Ok(format!(r#"<a:r><a:rPr lang="en-US" sz="{size}" b="{}" i="{}"><a:solidFill><a:srgbClr val="{}"/></a:solidFill><a:latin typeface="{}"/></a:rPr><a:t>{}</a:t></a:r>"#, bool_u8(run.bold), bool_u8(run.italic), xml::color(&run.color)?, xml::attr(&run.font_family), xml::text(&run.text)))
    }).collect::<Result<String, String>>()?;
    Ok(format!(
        r#"<a:p><a:pPr algn="{alignment}">{bullet}</a:pPr>{runs}<a:endParaRPr lang="en-US"/></a:p>"#
    ))
}

fn image_shape(
    id: usize,
    element: &PresentationElement,
    image: &PresentationImage,
    rid: &str,
    _target: &str,
) -> String {
    let f = element.frame;
    format!(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="{id}" name="{}" descr="{}"/><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#,
        xml::attr(&element.object_id),
        xml::attr(&image.alt_text),
        f.x,
        f.y,
        f.width,
        f.height
    )
}

fn chart_shape(
    id: usize,
    element: &PresentationElement,
    _chart: &PresentationChart,
    rid: &str,
) -> String {
    let f = element.frame;
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="{}"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="{rid}"/></a:graphicData></a:graphic></p:graphicFrame>"#,
        xml::attr(&element.object_id),
        f.x,
        f.y,
        f.width,
        f.height
    )
}

fn slide_relationships(
    slide_number: usize,
    layout_number: usize,
    resources: &SlideResources,
) -> String {
    let mut xml = format!(
        r#"{XML_HEAD}<Relationships xmlns="{REL_NS}">{}{}"#,
        relationship(
            "rId1",
            &format!("../slideLayouts/slideLayout{layout_number}.xml"),
            "slideLayout"
        ),
        relationship(
            "rId2",
            &format!("../notesSlides/notesSlide{slide_number}.xml"),
            "notesSlide"
        )
    );
    for (index, resource) in resources.relations.iter().enumerate() {
        xml.push_str(&relationship(
            &format!("rId{}", index + 3),
            &format!(
                "../{}",
                resource
                    .target
                    .strip_prefix("ppt/")
                    .unwrap_or(&resource.target)
            ),
            resource.kind,
        ));
    }
    xml.push_str("</Relationships>");
    xml
}

fn chart_xml(chart: &PresentationChart, _index: usize) -> Result<String, String> {
    let tag = match chart.chart_type {
        ChartType::Column | ChartType::Bar => "barChart",
        ChartType::Line => "lineChart",
        ChartType::Pie => "pieChart",
    };
    let grouping = if matches!(chart.chart_type, ChartType::Column | ChartType::Bar) {
        format!(
            r#"<c:barDir val="{}"/><c:grouping val="clustered"/>"#,
            if chart.chart_type == ChartType::Bar {
                "bar"
            } else {
                "col"
            }
        )
    } else {
        String::new()
    };
    let series = chart.series.iter().enumerate().map(|(index, series)| {
        let categories = chart.categories.iter().enumerate().map(|(i, value)| format!(r#"<c:pt idx="{i}"><c:v>{}</c:v></c:pt>"#, xml::text(value))).collect::<String>();
        let values = series.values.iter().enumerate().map(|(i, value)| format!(r#"<c:pt idx="{i}"><c:v>{value}</c:v></c:pt>"#)).collect::<String>();
        format!(r#"<c:ser><c:idx val="{index}"/><c:order val="{index}"/><c:tx><c:v>{}</c:v></c:tx><c:cat><c:strLit><c:ptCount val="{}"/>{categories}</c:strLit></c:cat><c:val><c:numLit><c:formatCode>General</c:formatCode><c:ptCount val="{}"/>{values}</c:numLit></c:val></c:ser>"#, xml::text(&series.name), chart.categories.len(), series.values.len())
    }).collect::<String>();
    let axes = if chart.chart_type == ChartType::Pie {
        String::new()
    } else {
        "<c:axId val=\"123456\"/><c:axId val=\"654321\"/>".to_string()
    };
    let axis_defs = if chart.chart_type == ChartType::Pie {
        String::new()
    } else {
        "<c:catAx><c:axId val=\"123456\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:axPos val=\"b\"/><c:crossAx val=\"654321\"/></c:catAx><c:valAx><c:axId val=\"654321\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:axPos val=\"l\"/><c:crossAx val=\"123456\"/></c:valAx>".to_string()
    };
    Ok(format!(
        r#"{XML_HEAD}<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:layout/><c:{tag}>{grouping}{series}{axes}</c:{tag}>{axis_defs}</c:plotArea><c:legend><c:legendPos val="r"/></c:legend><c:plotVisOnly val="1"/></c:chart></c:chartSpace>"#,
        xml::text(&chart.title)
    ))
}

fn notes_slide_xml(slide: &PresentationSlide, citations: &[PresentationCitation]) -> String {
    let mut note = slide.notes.speaker_notes.clone();
    for citation in citations
        .iter()
        .filter(|citation| citation.slide_id == slide.slide_id)
    {
        note.push_str("\n");
        note.push_str(&citation.label);
        if let Some(locator) = &citation.locator {
            note.push_str(" — ");
            note.push_str(locator);
        }
    }
    let block = TextBlock {
        paragraphs: vec![TextParagraph {
            runs: vec![TextRun {
                text: note,
                font_family: "Arial".to_string(),
                font_size_pt: 12.0,
                bold: false,
                italic: false,
                color: "202124".to_string(),
            }],
            alignment: TextAlignment::Left,
            bullet: false,
        }],
        vertical_alignment: VerticalAlignment::Top,
    };
    let shape = text_shape(
        2,
        "Speaker notes",
        Frame {
            x: 685_800,
            y: 4_572_000,
            width: 5_486_400,
            height: 3_657_600,
        },
        "rect",
        None,
        None,
        &block,
    )
    .unwrap_or_default();
    let mut tree = shape_tree_root();
    let insert = tree.len() - "</p:spTree>".len();
    tree.insert_str(insert, &shape);
    format!(
        r#"{XML_HEAD}<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{DOC_REL}" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld>{tree}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"#
    )
}

fn notes_master_xml() -> String {
    format!(
        r#"{XML_HEAD}<p:notesMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{DOC_REL}" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld>{}</p:cSld><p:clrMap accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" bg1="lt1" bg2="lt2" folHlink="folHlink" hlink="hlink" tx1="dk1" tx2="dk2"/><p:notesStyle/></p:notesMaster>"#,
        shape_tree_root()
    )
}

fn shape_tree_root() -> String {
    "<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree>".to_string()
}

fn relationships(values: &[(&str, &str, &str)]) -> String {
    let mut xml = format!(r#"{XML_HEAD}<Relationships xmlns="{REL_NS}">"#);
    for (id, target, kind) in values {
        xml.push_str(&relationship(id, target, kind));
    }
    xml.push_str("</Relationships>");
    xml
}

fn relationship(id: &str, target: &str, kind: &str) -> String {
    format!(
        r#"<Relationship Id="{id}" Type="{DOC_REL}/{kind}" Target="{}"/>"#,
        xml::attr(target)
    )
}

fn geometry_name(value: ShapeGeometry) -> &'static str {
    match value {
        ShapeGeometry::Rectangle => "rect",
        ShapeGeometry::RoundedRectangle => "roundRect",
        ShapeGeometry::Ellipse => "ellipse",
        ShapeGeometry::Triangle => "triangle",
        ShapeGeometry::Line => "line",
    }
}

fn bool_u8(value: bool) -> u8 {
    if value {
        1
    } else {
        0
    }
}
