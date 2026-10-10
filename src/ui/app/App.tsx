/** The app window (Tauri label `viewer`): shell + hash router, behind the first-run gate. */
import { useEffect, useState } from "react";
import { AppShell } from "./AppShell";
import { AppDataProvider } from "./appData";
import { migrateLegacyLink, useRoute } from "./router";
import { LibraryPage } from "./LibraryPage";
import { MarksPage } from "./MarksPage";
import { PlayerPage } from "../player/PlayerPage";
import { ExportPage } from "../export/ExportPage";
import { ExportsPage } from "../export/ExportsPage";
import { SettingsPage } from "../settings/SettingsPage";
import { FirstRun } from "../settings/FirstRun";
import { getSettings } from "../hooks/useApi";

migrateLegacyLink();

/**
 * Whether first run still has to happen: read once. Until the answer arrives
 * nothing renders; if settings cannot be read, the app opens rather than
 * trapping someone in first run.
 */
function useOnboarded(): [boolean | null, () => void] {
  const [onboarded, setOnboarded] = useState<boolean | null>(null);
  useEffect(() => {
    let alive = true;
    getSettings()
      .then((s) => alive && setOnboarded(s.onboarded))
      .catch(() => alive && setOnboarded(true));
    return () => {
      alive = false;
    };
  }, []);
  return [onboarded, () => setOnboarded(true)];
}

export function App() {
  const route = useRoute();
  const [onboarded, finishFirstRun] = useOnboarded();

  useEffect(() => {
    document.title = "ChirpTime";
  }, []);

  if (onboarded === null) return <div className="h-full bg-paper" />;
  if (!onboarded) {
    return (
      <AppDataProvider>
        <FirstRun onDone={finishFirstRun} />
      </AppDataProvider>
    );
  }

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
      page = <MarksPage />;
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
