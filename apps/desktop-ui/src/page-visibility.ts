import { createContext, useContext } from "react";

export const PageVisibilityContext = createContext(true);
export function usePageActive() { return useContext(PageVisibilityContext); }
