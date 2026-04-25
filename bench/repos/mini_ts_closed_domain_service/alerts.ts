export const ALERT_LEVELS = ["info", "critical"] as const;
export type AlertLevel = typeof ALERT_LEVELS[number];

const SEVERITY: Partial<Record<AlertLevel, number>> = {
  info: 1,
};

export function alertSeverity(level: AlertLevel): number {
  return SEVERITY[level]!.valueOf();
}
