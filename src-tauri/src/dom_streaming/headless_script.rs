pub(super) fn headless_dom_script() -> &'static str {
    r#"(() => {
      const clean = value => String(value ?? '').replace(/\s+/g, ' ').trim();
      const hiddenAncestor = element => {
        for (let node = element; node instanceof Element; node = node.parentElement) {
          const tag = node.tagName.toLowerCase();
          if (['script','style','noscript','template','svg','canvas','head'].includes(tag)) return true;
          if (['header','nav','footer'].includes(tag)) return true;
          const role = clean(node.getAttribute('role')).toLowerCase();
          if (['banner','navigation','contentinfo'].includes(role)) return true;
          const hints = `${node.id || ''} ${node.className || ''}`.toLowerCase();
          if (/(^|[\s_-])(cookie|consent|gdpr|newsletter|promo|promotional|advertisement|subscribe)([\s_-]|$)/.test(hints)) return true;
          if (node.hidden || node.getAttribute('aria-hidden') === 'true') return true;
          const style = getComputedStyle(node);
          if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) return true;
        }
        return false;
      };
      const visible = element => element instanceof Element && !hiddenAncestor(element);
      const unique = (values, limit) => {
        const seen = new Set();
        const output = [];
        for (const raw of values) {
          const value = clean(raw);
          if (!value || seen.has(value)) continue;
          seen.add(value);
          output.push(value);
          if (output.length >= limit) break;
        }
        return output;
      };
      const markdown = element => {
        const text = clean(element.innerText || element.textContent);
        const tag = element.tagName.toLowerCase();
        if (tag === 'h1') return `# ${text}`;
        if (tag === 'h2') return `## ${text}`;
        if (tag === 'h3') return `### ${text}`;
        if (['h4','h5','h6'].includes(tag)) return `#### ${text}`;
        if (['li','dt','dd'].includes(tag)) return `- ${text}`;
        if (tag === 'blockquote') return `> ${text}`;
        if (tag === 'pre') return `\`\`\`\n${text}\n\`\`\``;
        if (element.getAttribute('role') === 'heading') return `## ${text}`;
        return text;
      };
      let textNodes = Array.from(document.querySelectorAll(
        "h1,h2,h3,h4,h5,h6,p,li,dt,dd,blockquote,pre,figcaption,address,[role='heading']"
      )).filter(visible).map(markdown);
      if (textNodes.length < 240) {
        const directText = element => Array.from(element.childNodes)
          .filter(node => node.nodeType === Node.TEXT_NODE)
          .map(node => node.textContent)
          .join(' ');
        textNodes = textNodes.concat(Array.from(document.querySelectorAll('div,span'))
          .filter(visible).map(directText).filter(Boolean));
      }
      if (textNodes.length < 240) {
        textNodes = textNodes.concat(Array.from(document.querySelectorAll('[aria-label]'))
          .filter(visible).map(element => element.getAttribute('aria-label')).filter(Boolean));
      }
      let visibleText = unique(textNodes, 240).join('\n');
      if (!visibleText) visibleText = clean(document.body?.innerText || '');
      const inputs = Array.from(document.querySelectorAll('input,textarea,select'))
        .filter(visible).slice(0, 80).map(element => {
          const label = element.labels?.[0]?.innerText
            || element.closest('label')?.innerText
            || element.getAttribute('aria-label')
            || '';
          return {
            inputType: clean(element.getAttribute('type') || element.tagName.toLowerCase()).slice(0, 40),
            name: clean(element.getAttribute('name')).slice(0, 120),
            label: clean(label).slice(0, 320),
            placeholder: clean(element.getAttribute('placeholder')).slice(0, 320)
          };
        });
      const buttons = unique(Array.from(document.querySelectorAll(
        "button,[role='button'],input[type='submit'],input[type='button']"
      )).filter(visible).map(element => element.innerText || element.value || element.getAttribute('aria-label')), 100);
      const links = Array.from(document.querySelectorAll('a[href]')).filter(visible)
        .map(element => ({text: clean(element.innerText || element.textContent), url: element.href}))
        .filter(link => link.text && /^https?:/i.test(link.url)).slice(0, 120);
      const tables = Array.from(document.querySelectorAll('table')).filter(visible).slice(0, 12)
        .map(table => ({
          label: clean(table.querySelector('caption')?.innerText || table.getAttribute('aria-label')).slice(0, 320),
          rows: Array.from(table.querySelectorAll('tr')).slice(0, 40)
            .map(row => Array.from(row.querySelectorAll('th,td')).slice(0, 16)
              .map(cell => clean(cell.innerText || cell.textContent).slice(0, 320))
              .filter(Boolean))
            .filter(row => row.length)
        })).filter(table => table.rows.length);
      const temporalEvidence = [];
      const addTemporal = (value, evidenceType, label) => {
        value = clean(value || '').slice(0, 160);
        if (value && !temporalEvidence.some(item => item.value === value && item.evidenceType === evidenceType)) {
          temporalEvidence.push({value, evidenceType, label: clean(label || '').slice(0, 160)});
        }
      };
      document.querySelectorAll('meta[content]').forEach(element => {
        const key = (element.getAttribute('property') || element.getAttribute('name') || element.getAttribute('itemprop') || '').toLowerCase();
        if (key.includes('published') || key.includes('publication')) addTemporal(element.content, 'publicationDate', key);
        else if (key.includes('modified') || key.includes('updated')) addTemporal(element.content, 'updatedDate', key);
        else if (key.includes('release')) addTemporal(element.content, 'releaseDate', key);
      });
      document.querySelectorAll('time[datetime]').forEach(element => addTemporal(element.dateTime, 'publicationDate', element.textContent));
      return {
        url: location.href,
        title: document.title || '',
        visibleText: visibleText.slice(0, 36000),
        inputs,
        buttons,
        links,
        tables,
        temporalEvidence: temporalEvidence.slice(0, 24),
        extractionMethod: 'headless_browser'
      };
    })()"#
}
