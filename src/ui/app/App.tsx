/** The app window (Tauri label `viewer`): shell + hash router. */
import { useEffect } from "react";
import { AppShell } from "./AppShell";
import { AppDataProvider } from "./appData";
import { migrateLegacyLink, useRoute } from "./router";
import { LaterPage, LibraryPage } from "./pages";
import { PlayerPage } from "../player/PlayerPage";
import { fmtTimeAmPm } from "../session/format";

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
      page = (
        <LaterPage
          title="Export"
          line={
            route.from !== null && route.to !== null
              ? `Exporting ${fmtTimeAmPm(route.from)} – ${fmtTimeAmPm(route.to)} comes with the export composer`
              : "The export composer comes later"
          }
        />
      );
      break;
    case "marks":
      page = <LaterPage title="Marks" line="Every mark across your sessions, with notes, comes later" />;
      break;
    case "exports":
      page = <LaterPage title="Exports" line="Finished exports will be listed here later" />;
      break;
    case "settings":
      page = <LaterPage title="Settings" line="Settings move into this window later; the recorder still has its options" />;
      break;
  }

  return (
    <AppDataProvider>
      <AppShell route={route}>{page}</AppShell>
    </AppDataProvider>
  );
}
