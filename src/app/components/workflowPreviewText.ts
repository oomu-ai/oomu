export function firstSentenceForWorkflowPreview(value: string) {
  const trimmed = value.trim();

  for (let index = 0; index < trimmed.length; index += 1) {
    if (![".", "!", "?"].includes(trimmed[index])) {
      continue;
    }

    const next = trimmed[index + 1];
    if (next === undefined || /\s/.test(next)) {
      return trimmed.slice(0, index + 1);
    }
  }

  return trimmed;
}
