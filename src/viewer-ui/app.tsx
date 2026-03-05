import React, { useState, useEffect, useCallback } from "react";
import { createRoot } from "react-dom/client";
import { Layout } from "./components/Layout";
import { fetchDates, fetchDateEntries, fetchMonitorConfig } from "./hooks/useApi";
import type { AppState } from "./types";

function App() {
  const [state, setState] = useState<AppState>({
    dates: [],
    selectedDate: null,
    dateEntry: null,
    selectedTimestamp: null,
    monitorConfig: null,
    zoomedMonitor: null,
    loading: true,
  });

  // Load dates + monitor config on mount
  useEffect(() => {
    Promise.all([fetchDates(), fetchMonitorConfig()])
      .then(([dates, monitorConfig]) => {
        setState((s) => ({
          ...s,
          dates,
          monitorConfig,
          loading: false,
          // Auto-select first date
          selectedDate: dates[0] ?? null,
        }));
        // Auto-load first date
        if (dates[0]) {
          loadDate(dates[0]);
        }
      })
      .catch((err) => {
        console.error("Failed to load initial data:", err);
        setState((s) => ({ ...s, loading: false }));
      });
  }, []);

  const loadDate = useCallback(async (date: string) => {
    setState((s) => ({ ...s, loading: true }));
    try {
      const dateEntry = await fetchDateEntries(date);
      setState((s) => ({
        ...s,
        dateEntry,
        selectedDate: date,
        selectedTimestamp: dateEntry.entries[0]?.timestamp ?? null,
        zoomedMonitor: null,
        loading: false,
      }));
    } catch (err) {
      console.error("Failed to load date:", err);
      setState((s) => ({ ...s, loading: false }));
    }
  }, []);

  const onSelectDate = useCallback((date: string) => {
    loadDate(date);
  }, [loadDate]);

  const onSelectTimestamp = useCallback((ts: string) => {
    setState((s) => ({ ...s, selectedTimestamp: ts, zoomedMonitor: null }));
  }, []);

  const onSelectMonitor = useCallback((id: number | null) => {
    setState((s) => ({ ...s, zoomedMonitor: id }));
  }, []);

  return (
    <Layout
      state={state}
      onSelectDate={onSelectDate}
      onSelectTimestamp={onSelectTimestamp}
      onSelectMonitor={onSelectMonitor}
    />
  );
}

const root = createRoot(document.getElementById("root")!);
root.render(<App />);
