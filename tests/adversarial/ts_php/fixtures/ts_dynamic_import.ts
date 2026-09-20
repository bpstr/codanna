export async function run() {
    const { persist: save } = await import('./storage');
    save('invoice');
}
