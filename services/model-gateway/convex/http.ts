import { httpRouter } from "convex/server"
import { authComponent, createAuth } from "./auth"
import { completions, device, health, models } from "./gateway"
import { marketRoutes } from "./market/http"

const http = httpRouter()
authComponent.registerRoutes(http, createAuth)
http.route({ path: "/health", method: "GET", handler: health })
http.route({ path: "/device", method: "GET", handler: device })
http.route({ path: "/v1/models", method: "GET", handler: models })
http.route({ path: "/v1/chat/completions", method: "POST", handler: completions })
marketRoutes(http)
export default http
