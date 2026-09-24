import { trackError } from "../../telemetry";
import type { GraphDiagnostic } from "../../services";
import type { GraphController } from "./graph-class";

function diagnosticKey(diagnostic: GraphDiagnostic): string {
  return `${diagnostic.code}\u0000${diagnostic.location}\u0000${diagnostic.message}`;
}

export function setYamlDiagnostics(
  this: GraphController,
  diagnostics: GraphDiagnostic[],
): void {
  const previousKeys = new Set(this.yamlDiagnostics.map(diagnosticKey));
  const seen = new Set<string>();
  this.yamlDiagnostics = diagnostics.filter((diagnostic) => {
    const key = diagnosticKey(diagnostic);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  this.el.yamlWarning.hidden = this.yamlDiagnostics.length === 0;
  this.el.yamlWarning.open = false;
  this.el.yamlWarningList.replaceChildren();
  for (const diagnostic of this.yamlDiagnostics) {
    const item = document.createElement("li");
    item.textContent = `${diagnostic.location}: ${diagnostic.message}`;
    this.el.yamlWarningList.appendChild(item);
    if (!previousKeys.has(diagnosticKey(diagnostic))) {
      void trackError(
        "graph.fidelity",
        new Error(`[${diagnostic.code}] ${diagnostic.location}: ${diagnostic.message}`),
      );
    }
  }
}
