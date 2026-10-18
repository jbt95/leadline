def loops(items):
    for item in items:
        while item.ready:
            break


def for_else(items):
    for item in items:
        if item.ready:
            return item
    else:
        return None
