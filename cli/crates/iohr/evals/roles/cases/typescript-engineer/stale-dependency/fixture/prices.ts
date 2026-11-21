// Reads prices from the catalogue service.
import axios from "axios";

const api = axios.create({ baseURL: "https://catalogue.example.com", timeout: 5_000 });

export async function price(sku: string): Promise<number> {
  const { data } = await api.get<{ cents: number }>(`/prices/${encodeURIComponent(sku)}`);
  return data.cents;
}
