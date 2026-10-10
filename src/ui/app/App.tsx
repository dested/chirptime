/** The app window (Tauri label `viewer`): shell + hash router. */
import { useEffect } from "react";
import { AppShell } from "./AppShell";
import { AppDataProvider } from "./appData";
import { migrateLegacyLink, useRoute } from "./router";
import { LaterPage, LibraryPage } from "./pages";
import { PlayerPage } from "../player/PlayerPage";
import { ExportPage } from "../export/ExportPage";
import { ExportsPage } from "../export/ExportsPage";
import { SettingsPage } from "../settings/SettingsPage";

migrateLegacyLink();

export function App() {
  const route = useRoute();

  useEffect(() => {
    document.title = "ChirpTime";
  }, []);

  let page;
  switch (route.name) {
    case "library":
      page = <LibraryPage />;
      break;
    case "session":
      // keyed: a different session is a fresh player and engine
      page = <PlayerPage key={route.id} id={route.id} initialT={route.t} />;
      break;
    case "export":
      page = <ExportPage key={route.id} id={route.id} from={route.from} to={route.to} />;
      break;
    case "marks":
      page = <LaterPage title="Marks" line="Every mark across your sessions, with notes, comes later" />;
      break;
    case "exports":
      page = <ExportsPage />;
      break;
    case "settings":
      page = <SettingsPage />;
      break;
  }

  return (
    <AppDataProvider>
      <AppShell route={route}>{page}</AppShell>
    </AppDataProvider>
  );
}
