<?php
class Gateway {
    public function charge(): void {}
}
function pay(?Gateway $gateway): void {
    $gateway?->charge();
}
