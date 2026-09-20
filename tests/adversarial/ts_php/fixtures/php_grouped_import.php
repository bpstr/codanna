<?php
namespace App\Checkout;
use App\Billing\{Gateway, Receipt as Invoice};

function pay(Gateway $gateway): Invoice {
    return $gateway->charge();
}
