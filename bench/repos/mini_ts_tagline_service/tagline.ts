export type Profile = {
  segments?: string[] | null;
} | null;

export function primaryTagline(profile: Profile): string {
  return profile!.segments![0]!.trim();
}
