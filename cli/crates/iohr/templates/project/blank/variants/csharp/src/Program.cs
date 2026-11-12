namespace App;

public static class Greeting
{
    public static string Hello() => "hello from @@NAME@@";

}

public static class Program
{
    public static void Main() => Console.WriteLine(Greeting.Hello());
}
