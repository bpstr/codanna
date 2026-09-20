class Boxed {
    static make() { return 1; }
}
function make(value: unknown) {}

export function sameLine() { make(Boxed.make()); }

export function splitLines() {
    make(
        Boxed.make()
    );
}
