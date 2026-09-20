class First:
    def run(self):
        return "first"

class Second:
    def run(self):
        return "second"

class Service:
    def execute(self):
        worker = First()
        self.worker: Second = Second()
        return worker.run()

if __name__ == "__main__":
    assert Service().execute() == "first"
