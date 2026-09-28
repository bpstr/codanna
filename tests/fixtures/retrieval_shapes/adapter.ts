import { mergePage } from './page';

export function createAdapter(fetchPage: () => Promise<number[]>) {
  let current: number[] = [];
  return {
    async listColumn() {
      const incoming = await fetchPage();
      current = mergePage(current, incoming);
      return current;
    },
    async loadTasks() {
      const load = (async () => {
        const incoming = await fetchPage();
        current = mergePage(current, incoming);
        return current;
      })();
      return load;
    },
  };
}

// An ordinary named caller provides a positive resolver control.
export function testHelper() {
  return mergePage([], [1]);
}
