function persist(value: string): void {}
export const save = value => persist(value);
export const control = (value: string) => persist(value);
