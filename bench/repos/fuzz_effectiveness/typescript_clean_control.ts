export function routeJob(input: { kind: string; attempts: number }): string {
    return `${input.kind}:${input.attempts}`;
}

export function createCounter(): { push(value: number): number } {
    return {
        push(value: number): number {
            return value;
        },
    };
}
