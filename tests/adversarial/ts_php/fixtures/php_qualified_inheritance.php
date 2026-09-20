<?php
namespace Framework {
    class BaseController {}
    interface Action {}
}
namespace App {
    class Checkout extends \Framework\BaseController implements \Framework\Action {}
}
