import { Canvas } from "@react-three/fiber";
import type { ReactNode } from "react";

/**
 * The one WebGL canvas the 3D pages draw into (RFC 0217). It loads only with a page that asks for it,
 * as its own chunk, and with reduced motion it draws on demand: one frame per change, no animation loop.
 * The Planner's 3D view reuses it.
 */
export default function SceneCanvas({
  children,
  reduced,
  label,
  extent,
}: {
  children: ReactNode;
  reduced: boolean;
  label: string;
  extent: number;
}) {
  return (
    <div role="img" aria-label={label} className="h-full min-h-[320px] w-full">
      <Canvas
        frameloop={reduced ? "demand" : "always"}
        dpr={[1, 2]}
        camera={{ position: [0, extent * 0.9, extent * 1.1], fov: 45 }}
        gl={{ antialias: true }}
      >
        <ambientLight intensity={0.8} />
        <directionalLight position={[4, 8, 5]} intensity={1.4} />
        {children}
      </Canvas>
    </div>
  );
}
