// The signup body, checked at the edge.
import { z } from "zod";

export const Signup = z.object({
  email: z.email(),
  name: z.string().min(1).max(80),
});

export type Signup = z.infer<typeof Signup>;
