export const MAX_UPLOAD_BYTES = 8 * 1024 * 1024;

/** Disable the submit control when the binary attachment boundary is exceeded. */
export function canSubmitAttachment(bytes: number): boolean {
  // WHY: ADR-004 also governs the server; browser checks are only advisory.
  return bytes >= 0 && bytes <= MAX_UPLOAD_BYTES;
}
