import type { Grant } from './contract.ts';

export interface Route { destination: string; kind: 'app' | 'liveactivity' }
const routeWhere = `r.id = ? AND r.generation = ? AND p.id = ? AND p.epoch = ?
  AND i.id = ? AND i.epoch = ? AND r.destination_digest = ? AND r.environment = ?
  AND r.expires_at >= ? AND r.expires_at > ? AND r.revoked = 0`;
const bindings = (grant: Grant, now: number) => [grant.route_id, grant.generation, grant.pairing_id,
  grant.pairing_epoch, grant.installation_id, grant.installation_epoch, grant.destination_digest,
  grant.environment, grant.expires_at, now];
const routeFrom = 'routes r JOIN pairings p ON p.id = r.pairing_id JOIN installations i ON i.id = p.installation_id';
export async function lookupRoute(db: D1Database, grant: Grant, now: number): Promise<Route | null> {
  return db.prepare(`SELECT r.destination, r.kind FROM ${routeFrom} WHERE ${routeWhere}`)
    .bind(...bindings(grant, now)).first<Route>();
}
export async function charge(db: D1Database, grant: Grant, now: number, routine: boolean): Promise<boolean> {
  const day = Math.floor(now / 86400);
  try {
    const result = await db.prepare(`UPDATE installation_usage SET
      total = CASE WHEN day = ? THEN total ELSE 0 END + 1,
      routine = CASE WHEN day = ? THEN routine ELSE 0 END + ?, day = ?
      WHERE installation_id = ? AND day <= ? AND (day <> ? OR total < 1000)
      AND (day <> ? OR routine + ? <= 700)
      AND EXISTS (SELECT 1 FROM ${routeFrom} WHERE ${routeWhere})
      RETURNING total`)
      .bind(day, day, Number(routine), day, grant.installation_id, day, day, day, Number(routine), ...bindings(grant, now)).first();
    return result !== null;
  } catch (error) {
    if (error instanceof Error && error.message.includes('daily_quota')) return false;
    throw error;
  }
}
export async function invalidate(db: D1Database, grant: Grant): Promise<void> {
  await db.prepare('UPDATE routes SET revoked = 1 WHERE id = ? AND generation = ? AND destination_digest = ? AND revoked = 0')
    .bind(grant.route_id, grant.generation, grant.destination_digest).run();
}
