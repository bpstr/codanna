<?php
trait JsonSerializer {
    public function serialize(): string { return '{}'; }
}
trait CsvSerializer {
    public function serialize(): string { return ''; }
}
class Exporter {
    use JsonSerializer, CsvSerializer {
        JsonSerializer::serialize insteadof CsvSerializer;
        CsvSerializer::serialize as serializeCsv;
    }
    public function export(): array {
        return [$this->serialize(), $this->serializeCsv()];
    }
}
