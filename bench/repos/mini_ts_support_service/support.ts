export type Team = {
  contacts?: {
    supportEmail?: string | null;
  } | null;
} | null;

export function supportEmailDomain(team: Team): string {
  return team!.contacts!.supportEmail!.split("@")[1].toLowerCase();
}
