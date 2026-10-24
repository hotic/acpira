// History search helpers, DOM-free so the host tsconfig can type-check their test. The host searches the saved
// conversations with the same rule (every term, case-insensitive); titles and agent names are matched here

// The distinct lower-cased terms of a query; a session matches when it carries every one of them
export function searchTerms(query: string): string[] {
  return [...new Set(query.trim().toLowerCase().split(/\s+/).filter(Boolean))];
}

// The title / agent side of a search: every term must occur in one of the two
export function matchesTitle(title: string, agentName: string, terms: string[]): boolean {
  const hay = `${title.toLowerCase()}\n${agentName.toLowerCase()}`;
  return terms.every(term => hay.includes(term));
}

// Splits text around every occurrence of the terms (longest first, case-insensitive); matches sit at the odd indices
export function markParts(text: string, terms: string[]): string[] {
  if (!terms.length) return [text];
  const pattern = [...terms].sort((a, b) => b.length - a.length).map(term => term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|');
  return text.split(new RegExp(`(${pattern})`, 'giu'));
}
