def sum_list(data):
    """Sum all elements in a list."""
    total = 0
    for i in range(len(data)):
        total += data[i]
    return total

if __name__ == "__main__":
    assert sum_list([1, 2, 3]) == 6, f"expected 6, got {sum_list([1, 2, 3])}"
    assert sum_list([]) == 0, f"expected 0, got {sum_list([])}"
    assert sum_list([10]) == 10, f"expected 10, got {sum_list([10])}"
    assert sum_list([1, 1, 1, 1]) == 4
    print("PASS")
