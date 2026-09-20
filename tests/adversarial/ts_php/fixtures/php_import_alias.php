<?php
namespace App\Checkout;
use App\Billing\Gateway as Payments;

function pay(Payments $gateway): void {
    $gateway->charge();
}
