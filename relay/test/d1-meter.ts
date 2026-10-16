export interface Meter { rows_read: number; rows_written: number; statements: number }
export function measured(db: D1Database, meter: Meter): D1Database {
  const originals = new WeakMap<D1PreparedStatement, D1PreparedStatement>();
  const record = <T>(result: D1Result<T>): D1Result<T> => {
    meter.rows_read += result.meta.rows_read; meter.rows_written += result.meta.rows_written; meter.statements++;
    return result;
  };
  function statement(original: D1PreparedStatement): D1PreparedStatement {
    const wrapped = {
      bind(...values: unknown[]) { return statement(original.bind(...values)); },
      async first<T>(column?: string): Promise<T | null> {
        const result = record(await original.all<Record<string, unknown>>());
        const row = result.results[0]; return (row ? column ? row[column] : row : null) as T | null;
      },
      async all<T>() { return record(await original.all<T>()); },
      async run<T>() { return record(await original.run<T>()); },
      raw: original.raw.bind(original)
    } as D1PreparedStatement;
    originals.set(wrapped, original); return wrapped;
  }
  return {
    prepare(sql: string) { return statement(db.prepare(sql)); },
    async batch<T>(statements: D1PreparedStatement[]) {
      return (await db.batch<T>(statements.map(s => originals.get(s) ?? s))).map(record);
    },
    exec: db.exec.bind(db), dump: db.dump.bind(db), withSession: db.withSession.bind(db)
  } as D1Database;
}
