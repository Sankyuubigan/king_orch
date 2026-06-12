declare module "drawflow" {
  interface DrawflowConnection {
    node: string;
    output: string;
  }

  interface DrawflowInput {
    connections: DrawflowConnection[];
  }

  interface DrawflowOutput {
    connections: DrawflowConnection[];
  }

  export interface DrawflowNode {
    id: string;
    name: string;
    data: any;
    class: string;
    html: string;
    typenode: boolean;
    inputs: Record<string, DrawflowInput>;
    outputs: Record<string, DrawflowOutput>;
    pos_x: number;
    pos_y: number;
  }

  export interface DrawflowExport {
    drawflow: {
      Home: {
        data: Record<string, DrawflowNode>;
      };
    };
  }

  type DrawflowEvent = "nodeCreated" | "nodeSelected" | "nodeRemoved" | "connectionCreated" | "connectionRemoved" | "click";

  class Drawflow {
    constructor(element: HTMLElement, render?: any, options?: any);
    zoom: number;
    precanvas: HTMLElement;
    module: string;
    drawflow: {
      drawflow: {
        Home: {
          data: Record<string, DrawflowNode>;
        };
      };
    };
    start(): void;
    addNode(name: string, inputs: number, outputs: number, posX: number, posY: number, id?: string, data?: any, className?: string): string;
    addNodeImport(data: DrawflowNode, precanvas: HTMLElement): void;
    addNodeOutput(nodeId: string): void;
    removeNodeOutput(nodeId: string, outputKey: string): void;
    addConnection(fromId: string, toId: string, output: string, input: string): void;
    removeNodeId(nodeId: string): void;
    removeConnection(nodeId: string, inputKey: string): void;
    export(): DrawflowExport;
    import(data: DrawflowExport): void;
    getNodeFromId(id: string): DrawflowNode;
    dispatch(event: string, data?: any): void;
    on(event: DrawflowEvent, callback: (...args: any[]) => void): void;
    zoom_reset(): void;
    zoom_in(): void;
    zoom_out(): void;
  }

  export default Drawflow;
}
