"use client";
import { createContext, useContext } from "react";
import type { RasterBasemap } from "@/lib/basemap";

const MapConfig = createContext<RasterBasemap | undefined>(undefined);
export const MapConfigProvider = MapConfig.Provider;
/** Operator-configured raster basemap, if any (see GL_WEB_BASEMAP_URL). */
export const useRasterBasemap = () => useContext(MapConfig);
