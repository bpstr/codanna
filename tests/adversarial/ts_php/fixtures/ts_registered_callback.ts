function persist(value: string): void {}
export function run(values: string[]) {
    values.forEach(persist);
}
