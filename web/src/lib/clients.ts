import "server-only";
import { Client } from "@geoledger/client";
import { configuration } from "./config";
import { ClientPool } from "./client-pool";

// Survive module reloads in development; production runs one module instance.
const holder = globalThis as typeof globalThis & {
  __geoledgerClients?: ClientPool<Client>;
};
/** Shared per-credential GeoLedger clients for the console BFF. */
export function clients(): ClientPool<Client> {
  holder.__geoledgerClients ??= new ClientPool(
    (token) => new Client(configuration().endpoint, token),
    { max: 256, idleMs: 5 * 60 * 1000 },
  );
  return holder.__geoledgerClients;
}
