// Known outdated client: it still rejects payloads above seven mebibytes.
export function canSubmitAttachment(bytes: number): boolean {
  return bytes >= 0 && bytes <= 7 * 1024 * 1024;
}
