import React, { useState, useEffect, useCallback } from "react";
import { Layout } from "./components/Layout";
import { fetchDates, fetchDateEntries, fetchMonitorConfig, fetchOutputDir, getCaptureStatus } from "./hooks/useApi";
import type { AppState } from "./types";

export function App() {
  const [state, setState] = useState<AppState>({
    dates: [],
    selectedDate: null,
    dateEntry: null,
    selectedTimestamp: null,
    monitorConfig: null,
    zoomedMonitor: null,
    loading: true,
    captureStatus: {
      running: false,
      last_capture_time: null,
      next_capture_time: null,
      captures_today: 0,
    },
    outputDir: "",
  });

  useEffect(() => {
    Promise.all([fetchDates(), fetchMonitorConfig(), fetchOutputDir(), getCaptureStatus()])
      .then(([dates, monitorConfig, outputDir, captureStatus]) => {
        setState((s) => ({
          ...s,
          dates,
          monitorConfig,
          outputDir,
          captureStatus,
          loading: false,
          selectedDate: dates[0] ?? null,
        }));
        if (dates[0]) {
          loadDate(dates[0]);
        }
      })
      .catch((err) => {
        console.error("Failed to load initial data:", err);
        setState((s) => ({ ...s, loading: false }));
      });
  }, []);

  // Poll capture status every 5 seconds
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const status = await getCaptureStatus();
        setState((s) => ({ ...s, captureStatus: status }));
      } catch {}
    }, 5000);
    return () => clearInterval(interval);
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

  const refreshDates = useCallback(async () => {
    const dates = await fetchDates();
    setState((s) => ({ ...s, dates }));
    if (state.selectedDate) {
      loadDate(state.selectedDate);
    }
  }, [state.selectedDate, loadDate]);

  const onCaptureStatusChange = useCallback((status: AppState["captureStatus"]) => {
    setState((s) => ({ ...s, captureStatus: status }));
  }, []);

  return (
    <Layout
      state={state}
      onSelectDate={onSelectDate}
      onSelectTimestamp={onSelectTimestamp}
      onSelectMonitor={onSelectMonitor}
      onRefresh={refreshDates}
      onCaptureStatusChange={onCaptureStatusChange}
    />
  );
}
