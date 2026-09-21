def parse(value):
    try:
        if not value:
            raise ValueError("empty")
        return int(value)
    except ValueError:
        return 0
    else:
        return 1
    finally:
        print("done")
