<?php
class Gateway {
    public function charge(): void {}
}
class Checkout {
    public function __construct(private readonly Gateway $gateway) {}

    public function pay(): void {
        $this->gateway->charge();
    }
}
