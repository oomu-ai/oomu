type LocalizedDirective = {
  locale: string;
  pattern: RegExp;
  topicGroup: number;
};

const localizedFreshnessPatterns: RegExp[] = [
  /\b(?:heute|aktuell|neueste[nrsm]?)\b/iu,
  /\b(?:hoy|actual(?:es)?|m[aá]s\s+reciente)\b/iu,
  /\b(?:aujourd['’]hui|actuel(?:le|les|s)?|plus\s+r[eé]cent)\b/iu,
  /\b(?:hari\s+ini|terbaru|saat\s+ini)\b/iu,
  /(?:今日|現在|最新)/u,
  /\b(?:hoje|atual|mais\s+recente)\b/iu,
  /(?:^|[^\p{L}\p{N}_])(?:сегодня|текущ\p{L}*|последн\p{L}*)(?=$|[^\p{L}\p{N}_])/iu,
  /(?:^|[^\p{L}\p{N}_])(?:сьогодні|поточн\p{L}*|останн\p{L}*)(?=$|[^\p{L}\p{N}_])/iu,
  /\b(?:h[oô]m\s+nay|hiện\s+tại|mới\s+nhất)\b/iu,
  /(?:今天|目前|最新)/u,
];

const localizedDirectives: LocalizedDirective[] = [
  { locale: "de-DE", pattern: /^(?:bitte\s+)?suche\s+(?:im|bei|mit)\s+(?:google|duckduckgo|internet|web)\s+(?:nach\s+)?(.+)$/iu, topicGroup: 1 },
  { locale: "es-ES", pattern: /^(?:por\s+favor[,:]?\s+)?busca\s+(?:en|con)\s+(?:google|duckduckgo|internet|la\s+web)\s+(?:por\s+)?(.+)$/iu, topicGroup: 1 },
  { locale: "fr-FR", pattern: /^(?:s['’]il\s+vous\s+pla[iî]t[,:]?\s+)?(?:recherche|recherchez|cherche|cherchez)\s+(?:sur|avec)\s+(?:google|duckduckgo|internet|le\s+web)\s+(.+)$/iu, topicGroup: 1 },
  { locale: "id-ID", pattern: /^(?:tolong\s+)?cari\s+(?:di|dengan)\s+(?:google|duckduckgo|internet|web)\s+(.+)$/iu, topicGroup: 1 },
  { locale: "ja-JP", pattern: /^(?:google|duckduckgo|インターネット|ウェブ)(?:で|を使って)(.+?)(?:を)?検索(?:して|してください)?$/iu, topicGroup: 1 },
  { locale: "pt-BR", pattern: /^(?:por\s+favor[,:]?\s+)?pesquis(?:e|ar)\s+(?:no|na|com)\s+(?:google|duckduckgo|internet|web)\s+(?:por\s+)?(.+)$/iu, topicGroup: 1 },
  { locale: "ru-RU", pattern: /^(?:пожалуйста[,:]?\s+)?(?:найди|найдите|поищи|поищите)\s+(?:в|через)\s+(?:google|duckduckgo|интернете|сети)\s+(.+)$/iu, topicGroup: 1 },
  { locale: "uk-UA", pattern: /^(?:будь\s+ласка[,:]?\s+)?(?:знайди|знайдіть|пошукай|пошукайте)\s+(?:в|через)\s+(?:google|duckduckgo|інтернеті|мережі)\s+(.+)$/iu, topicGroup: 1 },
  { locale: "vi-VN", pattern: /^(?:vui\s+lòng\s+)?tìm\s+kiếm\s+(?:trên|bằng)\s+(?:google|duckduckgo|internet|web)\s+(?:về\s+)?(.+)$/iu, topicGroup: 1 },
  { locale: "zh-CN", pattern: /^(?:请)?(?:在|用)\s*(?:google|duckduckgo|互联网|网络)(?:上)?\s*搜索\s*(.+)$/iu, topicGroup: 1 },
  { locale: "zh-TW", pattern: /^(?:請)?(?:在|用)\s*(?:google|duckduckgo|網際網路|網路)(?:上)?\s*搜尋\s*(.+)$/iu, topicGroup: 1 },
];

export function localizedExplicitSearchQuery(content: string) {
  const normalized = content.trim();
  for (const directive of localizedDirectives) {
    const match = directive.pattern.exec(normalized);
    const topic = match?.[directive.topicGroup]?.trim();
    if (topic) {
      return topic.replace(/^[\s"'“”‘’]+|[\s"'“”‘’。！？!?]+$/gu, "").trim();
    }
  }
  return "";
}

export function hasLocalizedFreshnessIntent(content: string) {
  const normalized = content.trim();
  return Boolean(normalized) && localizedFreshnessPatterns.some((pattern) => pattern.test(normalized));
}
