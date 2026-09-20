# Case 1: breadth-first distance is not Python MRO.
class A:
    def run(self):
        return "A"

class X(A):
    pass

class Y:
    def run(self):
        return "Y"

class D(X, Y):
    def via_super(self):
        return super().run()

def execute(value: D):
    return value.run()

assert execute(D()) == "A"
assert D().via_super() == "A"

# Case 2: depth-first concatenation is not C3 either.
class Root:
    def run(self):
        return "Root"

class Left(Root):
    pass

class Right(Root):
    def run(self):
        return "Right"

class Diamond(Left, Right):
    pass

assert Diamond().run() == "Right"
