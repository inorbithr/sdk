using App;

// The entry point: reads HOST and PORT, starts the service and stops it cleanly on SIGINT or SIGTERM.
var log = new JsonLog(Console.Out);
Config cfg;
try
{
    cfg = Config.Load(Environment.GetEnvironmentVariable);
}
catch (ArgumentException e)
{
    log.Error("bad configuration", ("error", e.Message));
    return 2;
}

var service = new Service(log);
try
{
    await service.StartAsync(cfg);
}
catch (IOException e)
{
    log.Error("cannot listen", ("error", e.GetType().Name));
    return 1;
}

await service.WaitAsync();
await service.StopAsync();
return 0;
