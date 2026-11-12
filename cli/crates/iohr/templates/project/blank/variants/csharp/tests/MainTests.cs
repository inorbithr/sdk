using App;

// One test, no framework to install: exits 1 when it fails.
if (Greeting.Hello() != "hello from @@NAME@@")
{
    Console.Error.WriteLine("FAIL: Hello() returned " + Greeting.Hello());
    return 1;
}
Console.WriteLine("ok: hello");
return 0;
