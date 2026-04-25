import type { AlertChannel } from "./notification-types.ts";

const LABELS: Partial<Record<AlertChannel, string>> = {
  email: "Email",
};

export function channelLabel(channel: AlertChannel): string {
  return LABELS[channel]!.toLowerCase();
}
