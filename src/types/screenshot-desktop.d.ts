declare module "screenshot-desktop" {
  interface Display {
    id: string | number;
    name?: string;
    width?: number;
    height?: number;
    left?: number;
    top?: number;
  }

  function screenshot(options?: { screen?: string | number; format?: string }): Promise<Buffer>;

  namespace screenshot {
    function all(): Promise<Buffer[]>;
    function listDisplays(): Promise<Display[]>;
  }

  export = screenshot;
}
