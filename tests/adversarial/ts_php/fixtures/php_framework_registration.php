<?php
use App\Contracts\Gateway;
use App\Billing\StripeGateway;
use App\Http\Controllers\CheckoutController;

$app->bind(Gateway::class, StripeGateway::class);
Route::post('/checkout', [CheckoutController::class, 'store']);
