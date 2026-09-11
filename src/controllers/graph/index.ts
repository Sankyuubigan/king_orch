import { GraphController } from "./graph-class";
import * as lifecyclePart from "./lifecycle-part";
import * as canvasPart from "./canvas-part";
import * as menuPart from "./menu-part";
import * as flowsPart from "./flows-part";
import * as historyPart from "./history-part";
import * as nodesPart from "./nodes-part";
import * as panelPart from "./panel-part";
import * as edgePart from "./edge-part";
import * as switchLogicPart from "./switch-logic-part";
import * as layoutModule from "./layout";
import * as nodeHtmlModule from "./node-html";

Object.assign(GraphController.prototype, {
  ...lifecyclePart,
  ...canvasPart,
  ...menuPart,
  ...flowsPart,
  ...historyPart,
  ...nodesPart,
  ...panelPart,
  ...edgePart,
  ...switchLogicPart,
  ...layoutModule,
  ...nodeHtmlModule,
});

export { GraphController } from "./graph-class";
export type { GraphElements } from "./types";