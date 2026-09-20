events = []

def register(fn):
    events.append("registered")
    return fn

def bootstrap():
    events.append("bootstrapped")
    return "configuration"

def send():
    events.append("sent")

@register
def handle(config=bootstrap()):
    send()

assert events == ["bootstrapped", "registered"]
