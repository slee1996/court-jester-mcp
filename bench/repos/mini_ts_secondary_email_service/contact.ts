export type Account = {
  contacts?: {
    emails?: string[] | null;
  } | null;
} | null;

export function secondarySupportEmail(account: Account): string {
  return account!.contacts!.emails![1]!.trim().toLowerCase();
}
