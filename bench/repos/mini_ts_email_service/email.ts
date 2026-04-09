export type Account = {
  emails?: string[] | null;
} | null;

export function primaryEmailDomain(account: Account): string {
  return account!.emails![0].split("@")[1].toLowerCase();
}
