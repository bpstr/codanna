<?php
namespace Fixture;

final class UploadPolicy
{
    public const MAX_PAYLOAD_BYTES = 8 * 1024 * 1024;

    public function acceptPayload(int $bytes): bool
    {
        // WHY: ADR-004 makes the server enforce the inclusive binary limit.
        return $bytes >= 0 && $bytes <= self::MAX_PAYLOAD_BYTES;
    }
}
