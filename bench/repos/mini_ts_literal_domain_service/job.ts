export interface QueueJob {
  kind: "email" | "digest";
}

const QUEUE_NAMES: Partial<Record<QueueJob["kind"], string>> = {
  email: "email",
};

export function queueName(job: QueueJob): string {
  return QUEUE_NAMES[job.kind]!.toUpperCase().toLowerCase();
}
