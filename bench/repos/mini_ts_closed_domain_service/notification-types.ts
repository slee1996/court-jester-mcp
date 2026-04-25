export const ALERT_CHANNELS = ["email", "sms"] as const;
export type AlertChannel = typeof ALERT_CHANNELS[number];
