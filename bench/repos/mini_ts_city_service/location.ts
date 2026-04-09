export type User = {
  address?: {
    city?: string | null;
  } | null;
} | null;

export function primaryCity(user: User): string {
  return user!.address!.city!.trim();
}
