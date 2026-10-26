import { orchestratorFetch } from "../../utils/orchestratorFetch"

export default defineEventHandler((event) => orchestratorFetch(event, "/cdc/brokers"))
