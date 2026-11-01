const MB = 1024 * 1024;

export function humanBytes(n: number): string {
  if (n >= 10 * MB) return `${Math.round(n / MB)} MB`;
  if (n >= MB) return `${(n / MB).toFixed(1)} MB`;
  return `${Math.round(n / 1024)} KB`;
}
