#[allow(unused_imports)]
pub(crate) use crate::dom_sanitizer::{element_is_boilerplate, semantic_markdown_block};

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::{Html, Selector};

    #[test]
    fn boilerplate_filter_drops_navigation_and_cookie_promotions() {
        let page = Html::parse_document(
            r#"<body>
              <nav><p>Navigation canary</p></nav>
              <section class="cookie-consent"><p>Cookie canary</p></section>
              <main><h2>Useful content</h2></main>
            </body>"#,
        );
        let selector = Selector::parse("p,h2").unwrap();
        let elements = page.select(&selector).collect::<Vec<_>>();
        assert!(element_is_boilerplate(elements[0]));
        assert!(element_is_boilerplate(elements[1]));
        assert!(!element_is_boilerplate(elements[2]));
    }

    #[test]
    fn semantic_blocks_are_flat_markdown() {
        let page =
            Html::parse_document("<body><h1>Overview</h1><ul><li>First option</li></ul></body>");
        let selector = Selector::parse("h1,li").unwrap();
        let elements = page.select(&selector).collect::<Vec<_>>();
        assert_eq!(
            semantic_markdown_block(elements[0], "Overview"),
            "# Overview"
        );
        assert_eq!(
            semantic_markdown_block(elements[1], "First option"),
            "- First option"
        );
    }
}
