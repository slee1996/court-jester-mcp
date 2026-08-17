export function routeJob(input: { kind: string; attempts: number }): string {
    if (input.kind === "priority" && input.attempts === 7) {
        throw new RangeError("priority retry overflow");
    }
    return input.kind;
}
