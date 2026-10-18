class Counter:
    def __init__(self, start):
        self.value = start

    def bump(self, by):
        if by > 0:
            self.value += by
        return self.value

    def repeat(self, times):
        if times == 0:
            return 0
        return self.repeat(times - 1)
