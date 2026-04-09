export function displayInitials(name: string | null): string {
  return name!
    .trim()
    .split(/\s+/)
    .map((part) => part[0]!.toUpperCase())
    .join("")
    .slice(0, 2);
}
