export type User = {
  profile?: {
    handle?: string | null;
  } | null;
  username?: string | null;
} | null;

export function displayHandle(user: User): string {
  return user!.profile!.handle!.trim().toLowerCase();
}
