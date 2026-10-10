/** Export composer (S5). Lead placeholder: `export` owns this file. */
export function ExportPage({ id, from, to }: { id: string; from: number | null; to: number | null }) {
  void id;
  void from;
  void to;
  return (
    <div className="px-8 pt-6">
      <h1 className="text-[24px] font-bold tracking-[-0.02em]">Export</h1>
      <p className="mt-2 text-ink-2">The export composer is being built</p>
    </div>
  );
}
