use super::{xml, PlaceholderKind, PresentationTheme, SlideLayout, SlideMaster, SlidePlaceholder};
use std::collections::HashMap;

const XML_HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const DOC_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

pub(crate) fn theme_xml(theme: &PresentationTheme) -> Result<String, String> {
    let c = &theme.colors;
    let color = |value: &str| xml::color(value);
    Ok(format!(
        r#"{XML_HEAD}<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="{}"><a:themeElements><a:clrScheme name="OOMU"><a:dk1><a:srgbClr val="{}"/></a:dk1><a:lt1><a:srgbClr val="{}"/></a:lt1><a:dk2><a:srgbClr val="{}"/></a:dk2><a:lt2><a:srgbClr val="{}"/></a:lt2><a:accent1><a:srgbClr val="{}"/></a:accent1><a:accent2><a:srgbClr val="{}"/></a:accent2><a:accent3><a:srgbClr val="{}"/></a:accent3><a:accent4><a:srgbClr val="{}"/></a:accent4><a:accent5><a:srgbClr val="{}"/></a:accent5><a:accent6><a:srgbClr val="{}"/></a:accent6><a:hlink><a:srgbClr val="{}"/></a:hlink><a:folHlink><a:srgbClr val="{}"/></a:folHlink></a:clrScheme><a:fontScheme name="OOMU"><a:majorFont><a:latin typeface="{}"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="{}"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="OOMU"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="25400"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="38100"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#,
        xml::attr(&theme.name),
        color(&c.dark)?,
        color(&c.light)?,
        color(&c.dark)?,
        color(&c.light)?,
        color(&c.accent_1)?,
        color(&c.accent_2)?,
        color(&c.accent_3)?,
        color(&c.accent_4)?,
        color(&c.accent_1)?,
        color(&c.accent_2)?,
        color(&c.hyperlink)?,
        color(&c.accent_3)?,
        xml::attr(&theme.fonts.heading),
        xml::attr(&theme.fonts.body)
    ))
}

pub(crate) fn master_xml(
    master: &SlideMaster,
    layouts: &HashMap<&str, usize>,
) -> Result<String, String> {
    let ids = master
        .layout_ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let layout = layouts
                .get(id.as_str())
                .ok_or_else(|| format!("Unknown layout {id}."))?;
            Ok(format!(
                r#"<p:sldLayoutId id="{}" r:id="rId{}"/>"#,
                2_147_483_649_u64 + *layout as u64,
                index + 1
            ))
        })
        .collect::<Result<String, String>>()?;
    Ok(format!(
        r#"{XML_HEAD}<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{DOC_REL}" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld name="{}">{}</p:cSld><p:clrMap accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" bg1="lt1" bg2="lt2" folHlink="folHlink" hlink="hlink" tx1="dk1" tx2="dk2"/><p:sldLayoutIdLst>{ids}</p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>"#,
        xml::attr(&master.name),
        shape_tree_root()
    ))
}

pub(crate) fn master_relationships(master: &SlideMaster, layouts: &HashMap<&str, usize>) -> String {
    let mut output = format!(r#"{XML_HEAD}<Relationships xmlns="{REL_NS}">"#);
    for (index, layout_id) in master.layout_ids.iter().enumerate() {
        output.push_str(&relationship(
            &format!("rId{}", index + 1),
            &format!(
                "../slideLayouts/slideLayout{}.xml",
                layouts[layout_id.as_str()]
            ),
            "slideLayout",
        ));
    }
    output.push_str(&relationship(
        &format!("rId{}", master.layout_ids.len() + 1),
        "../theme/theme1.xml",
        "theme",
    ));
    output.push_str("</Relationships>");
    output
}

pub(crate) fn layout_xml(layout: &SlideLayout) -> String {
    let mut tree = shape_tree_root();
    let insert = tree.len() - "</p:spTree>".len();
    let placeholders = layout
        .placeholders
        .iter()
        .enumerate()
        .map(|(index, value)| placeholder_xml(index + 2, value))
        .collect::<String>();
    tree.insert_str(insert, &placeholders);
    format!(
        r#"{XML_HEAD}<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="{DOC_REL}" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="cust" preserve="1"><p:cSld name="{}">{tree}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"#,
        xml::attr(&layout.name)
    )
}

fn placeholder_xml(id: usize, placeholder: &SlidePlaceholder) -> String {
    let kind = match placeholder.kind {
        PlaceholderKind::Title => "title",
        PlaceholderKind::Subtitle => "subTitle",
        PlaceholderKind::Body => "body",
        PlaceholderKind::Picture => "pic",
        PlaceholderKind::Chart => "chart",
        PlaceholderKind::Table => "tbl",
        PlaceholderKind::Footer => "ftr",
        PlaceholderKind::SlideNumber => "sldNum",
    };
    let f = placeholder.frame;
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{}"/><p:cNvSpPr/><p:nvPr><p:ph type="{kind}"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody></p:sp>"#,
        xml::attr(&placeholder.placeholder_id),
        f.x,
        f.y,
        f.width,
        f.height
    )
}

fn relationship(id: &str, target: &str, kind: &str) -> String {
    format!(
        r#"<Relationship Id="{id}" Type="{DOC_REL}/{kind}" Target="{}"/>"#,
        xml::attr(target)
    )
}

fn shape_tree_root() -> String {
    "<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree>".to_string()
}
