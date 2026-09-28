import { mergePage } from './page';

// Unlike the returned object, this object is inside a variable initializer.
export const adapter = {
  async listBoundColumn() {
    return mergePage([], [2]);
  },
};

// A namesake must not replace the imported implementation's graph target.
export function unrelated() {
  function mergePage() { return [99]; }
  return mergePage();
}
