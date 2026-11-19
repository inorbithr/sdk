using System.Collections;
using System.Text;
using App;

// The passes-on-init test, with no test framework to install: it boots the service on a free
// port, calls the probes, and checks the span, the trace id and what the log must not hold.
// It exits 1 with the first failure.

static void Check(bool ok, string what)
{
    if (!ok)
    {
        Console.Error.WriteLine("FAIL: " + what);
        Environment.Exit(1);
    }
}

using var http = new HttpClient();

async Task<(Service Svc, string Base, StringWriter Logs)> Boot()
{
    var logs = new StringWriter();
    var svc = new Service(new JsonLog(logs));
    var port = await svc.StartAsync(new Config("127.0.0.1", 0));
    return (svc, $"http://127.0.0.1:{port}", logs);
}

async Task<HttpResponseMessage> Get(string url, params (string, string)[] headers)
{
    var req = new HttpRequestMessage(HttpMethod.Get, url);
    foreach (var (k, v) in headers)
    {
        req.Headers.TryAddWithoutValidation(k, v);
    }
    return await http.SendAsync(req);
}

static int Count(string text, string needle)
{
    var n = 0;
    for (var i = text.IndexOf(needle, StringComparison.Ordinal); i >= 0; i = text.IndexOf(needle, i + 1, StringComparison.Ordinal))
    {
        n++;
    }
    return n;
}

// Probes answer.
{
    var (svc, url, _) = await Boot();
    try
    {
        Check((int)(await Get(url + "/healthz")).StatusCode == 200, "/healthz is 200");
        Check((int)(await Get(url + "/readyz")).StatusCode == 200, "/readyz is 200");
        var hello = await Get(url + "/v1/hello?name=Ada");
        Check((int)hello.StatusCode == 200 && (await hello.Content.ReadAsStringAsync()).Contains("Hello, Ada"), "hello answers");
        Check((int)(await Get(url + "/nope")).StatusCode == 404, "an unknown path is 404");
        Check((int)(await Get(url + "/v1/hello?name=" + new string('x', 65))).StatusCode == 400, "a name over 64 characters is 400");
    }
    finally
    {
        await svc.StopAsync();
    }
}

// One span with the route, the trace id in the log, and no secret in it.
{
    var (svc, url, logs) = await Boot();
    const string trace = "4bf92f3577b34da6a3ce929d0e0e4736";
    try
    {
        var r = await Get(url + "/v1/hello?name=secret-query-value",
            ("traceparent", $"00-{trace}-00f067aa0ba902b7-01"), ("Authorization", "Bearer secret-header-value"));
        var tp = r.Headers.GetValues("traceparent").FirstOrDefault() ?? "";
        Check(tp.StartsWith($"00-{trace}-", StringComparison.Ordinal), "the response continues the trace: " + tp);
    }
    finally
    {
        await svc.StopAsync();
    }
    var all = logs.ToString();
    var lines = all.Trim().Split('\n').Select(l => l.Trim()).ToArray();
    foreach (var line in lines)
    {
        Check(line.StartsWith('{') && line.EndsWith('}'), "a log line is JSON: " + line);
    }
    var span = lines.Single(l => l.Contains("\"msg\":\"span\""));
    var request = lines.Single(l => l.Contains("\"msg\":\"request\""));
    Check(span.Contains("\"route\":\"/v1/hello\"") && span.Contains($"\"trace_id\":\"{trace}\"")
        && span.Contains("\"parent_span_id\":\"00f067aa0ba902b7\""), "the span is right: " + span);
    Check(request.Contains($"\"trace_id\":\"{trace}\""), "the request log has the trace id");
    Check(Count(all, "\"msg\":\"span\"") == 1 && Count(all, "\"msg\":\"request\"") == 1, "one span and one request line");
    foreach (var secret in new[] { "secret-query-value", "secret-header-value" })
    {
        Check(!all.Contains(secret), "the log holds " + secret);
    }
    foreach (DictionaryEntry e in Environment.GetEnvironmentVariables())
    {
        var v = e.Value?.ToString() ?? "";
        Check(v.Length < 12 || !all.Contains(v), "the log holds the value of " + e.Key);
    }
}

// An unknown path is never logged, and metrics count by route.
{
    var (svc, url, logs) = await Boot();
    string metrics;
    try
    {
        await Get(url + "/private/user-1234");
        await Get(url + "/healthz");
        metrics = await (await Get(url + "/metrics")).Content.ReadAsStringAsync();
    }
    finally
    {
        await svc.StopAsync();
    }
    Check(!logs.ToString().Contains("user-1234"), "the log holds a raw path");
    Check(metrics.Contains("route=\"unmatched\",status=\"404\""), "metrics count unmatched: " + metrics);
    Check(metrics.Contains("route=\"/healthz\",status=\"200\""), "metrics count /healthz: " + metrics);
}

// Configuration.
{
    var refused = false;
    try
    {
        Config.Load(k => k == "PORT" ? "99999" : null);
    }
    catch (ArgumentException)
    {
        refused = true;
    }
    Check(refused, "a port out of range was accepted");
    var c = Config.Load(_ => null);
    Check(c is { Port: 8080, Host: "127.0.0.1" }, "defaults");
}

Console.WriteLine("ok: 4 checks passed");
return 0;
