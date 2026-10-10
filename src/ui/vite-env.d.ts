/// <reference types="vite/client" />

interface Window {
  /** DEV only: the player's engine, for the browser harness (`?mock=1`) */
  __chirpEngine?: import("./session/engine").SyncEngine;
}
