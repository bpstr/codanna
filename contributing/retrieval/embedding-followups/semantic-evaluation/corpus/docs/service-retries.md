# Service retries

Waystation's synthetic transport policy distinguishes temporary service failures from invalid inputs. A transient unavailable response may be retried within a fixed attempt budget. The wait grows after each failed attempt, and the operation stops when the budget is exhausted.

An authentication failure, an incompatible vector width, or an input rejected for excessive length is reported immediately. Repeating the identical invalid request does not correct it. Error diagnostics include the stage, source identifier when available, and the number of attempts, while excluding document text and credentials.

This policy concerns failed transport requests. It does not combine watcher events, choose passage boundaries, publish collections, or determine which search matches are eligible. The notes describe a fictional policy; they contain no endpoints or instructions to contact a service.
