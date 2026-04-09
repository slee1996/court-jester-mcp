export type Profile = {
  preferences?: {
    timezone?: string | null;
  } | null;
} | null;

export function preferredTimezone(profile: Profile): string {
  return profile!.preferences!.timezone!.trim();
}
