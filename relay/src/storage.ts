import type { Grant } from './contract.ts';

export interface Route { destination: string; kind: 'app' | 'liveactivity'; expires_at: number }
const routeWhere = `r.id = ? AND r.generation = ? AND p.id = ? AND p.epoch = ?
  AND i.id = ? AND i.epoch = ? AND r.destination_digest = ? AND r.environment = ?
  AND r.expires_at >= ? AND ? > MAX(?, unixepoch()) AND r.revoked = 0`;
const bindings = (grant: Grant, now: number) => [grant.route_id, grant.generation, grant.pairing_id,
  grant.pairing_epoch, grant.installation_id, grant.installation_epoch, grant.destination_digest,
  grant.environment, grant.expires_at, grant.expires_at, now];
const routeFrom = 'routes r JOIN pairings p ON p.id = r.pairing_id JOIN installations i ON i.id = p.installation_id';
export async function lookupRoute(db: D1Database, grant: Grant, now: number): Promise<Route | null> {
  return db.prepare(`SELECT r.destination, r.kind, r.expires_at FROM ${routeFrom} WHERE ${routeWhere}`)
    .bind(...bindings(grant, now)).first<Route>();
}
export async function charge(db: D1Database, grant: Grant, now: number, routine: boolean): Promise<'charged' | 'invalid_route' | 'daily_quota' | 'unavailable'> {
  const day = Math.floor(now / 86400);
  const currentDay = 'MAX(?, CAST(unixepoch() / 86400 AS INTEGER))';
  try {
    const result = await db.prepare(`UPDATE installation_usage SET
      total = CASE WHEN day = ${currentDay} THEN total ELSE 0 END + 1,
      routine = CASE WHEN day = ${currentDay} THEN routine ELSE 0 END + ?, day = ${currentDay}
      WHERE installation_id = ? AND day <= ${currentDay} AND (day <> ${currentDay} OR total < 1000)
      AND (day <> ${currentDay} OR routine + ? <= 700)
      AND EXISTS (SELECT 1 FROM ${routeFrom} WHERE ${routeWhere})
      RETURNING total`)
      .bind(day, day, Number(routine), day, grant.installation_id, day, day, day, Number(routine), ...bindings(grant, now)).first();
    if (result !== null) return 'charged';
  } catch (error) {
    if (!(error instanceof Error) || !error.message.includes('daily_quota')) throw error;
  }
  if (!await lookupRoute(db, grant, now)) return 'invalid_route';
  const usage = await db.prepare('SELECT 1 AS present FROM installation_usage WHERE installation_id = ?').bind(grant.installation_id).first();
  return usage ? 'daily_quota' : 'unavailable';
}
export async function invalidate(db: D1Database, grant: Grant): Promise<void> {
  await db.prepare('UPDATE routes SET revoked = 1 WHERE id = ? AND generation = ? AND destination_digest = ? AND revoked = 0')
    .bind(grant.route_id, grant.generation, grant.destination_digest).run();
}
