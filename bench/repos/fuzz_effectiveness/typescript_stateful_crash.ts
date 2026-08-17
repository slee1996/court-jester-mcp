export function createCounter() {
    let calls = 0;
    function push(value: number): number {
        calls += 1;
        if (calls === 2) {
            throw new ReferenceError("counter second-step crash");
        }
        return value;
    }
    return { push };
}
