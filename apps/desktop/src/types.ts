export interface Device {
  id: string;
  name: string;
  port: number;
}
export interface Peer {
  id: string;
  name: string;
  address: string;
}
export interface DiscoveredDevice {
  id: string;
  name: string;
  addresses: string[];
}
export interface Pairing {
  id: string;
  peer: Peer;
  direction: string;
  code: string;
  status: string;
  error: string | null;
}
export interface Activity {
  id: string;
  peerId: string;
  direction: string;
  kind: string;
  content: string;
  status: string;
  size: number;
  transferred: number;
  path: string | null;
  error: string | null;
  timestamp: number;
}
export interface Snapshot {
  local: Device;
  listenAddress: string;
  localAddresses: string[];
  downloadDir: string;
  discovered: DiscoveredDevice[];
  discoveryError: string | null;
  peers: Peer[];
  pairings: Pairing[];
  activities: Activity[];
}
