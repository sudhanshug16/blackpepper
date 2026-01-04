export function normalizePath(value: string): string {
  const normalized = value.replace(/\\/g, "/");
  if (normalized === "/") return "/";
  return normalized.replace(/\/+$/u, "");
}

export function joinPath(...parts: string[]): string {
  const filtered = parts.filter((part) => part.length > 0);
  if (filtered.length === 0) return "";

  const normalized = filtered.map((part, index) => {
    if (part === "/") return "/";
    const replaced = normalizePath(part);
    if (index === 0) return replaced;
    return replaced.replace(/^\/+/, "");
  });

  return normalized.reduce((acc, part) => {
    if (!acc) return part;
    if (acc === "/") return `/${part}`;
    return `${acc}/${part}`;
  }, "");
}
