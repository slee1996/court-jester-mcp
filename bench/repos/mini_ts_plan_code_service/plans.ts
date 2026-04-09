import { normalizePlanCode } from "./normalizers.ts";

export type Account = {
  plans?: Array<string | null> | null;
} | null;

export function primaryPlanCode(account: Account): string {
  return normalizePlanCode(account!.plans![0]);
}
