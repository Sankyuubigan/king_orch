# Vendored копия benchmark/refactor_tools.py из Aider-AI/aider
# (без зависимостей от пакета aider). Используется проверкой рефакторинга.
import ast
from pathlib import Path


class ParentNodeTransformer(ast.NodeTransformer):
    def generic_visit(self, node):
        for child in ast.iter_child_nodes(node):
            child.parent = node
        return super(ParentNodeTransformer, self).generic_visit(node)


def verify_full_func_at_top_level(tree, func, func_children):
    func_nodes = [
        item for item in ast.walk(tree) if isinstance(item, ast.FunctionDef) and item.name == func
    ]
    assert func_nodes, f"Function {func} not found"

    for func_node in func_nodes:
        if not isinstance(func_node.parent, ast.Module):
            continue

        num_children = sum(1 for _ in ast.walk(func_node))
        pct_diff_children = abs(num_children - func_children) * 100 / func_children
        assert (
            pct_diff_children < 10
        ), f"Old method had {func_children} children, new method has {num_children}"
        return

    assert False, f"{func} is not a top level function"


def verify_old_class_children(tree, old_class, old_class_children):
    node = next(
        (
            item
            for item in ast.walk(tree)
            if isinstance(item, ast.ClassDef) and item.name == old_class
        ),
        None,
    )
    assert node is not None, f"Old class {old_class} not found"

    num_children = sum(1 for _ in ast.walk(node))

    pct_diff_children = abs(num_children - old_class_children) * 100 / old_class_children
    assert (
        pct_diff_children < 10
    ), f"Old class had {old_class_children} children, new class has {num_children}"


def verify_refactor(fname, func, func_children, old_class, old_class_children):
    with open(fname, "r") as file:
        file_contents = file.read()
    tree = ast.parse(file_contents)
    ParentNodeTransformer().visit(tree)

    verify_full_func_at_top_level(tree, func, func_children)
    verify_old_class_children(tree, old_class, old_class_children - func_children)