namespace InOrbit.Sdk;

/// <summary>
/// A profile: one credential's view of the API. A generated surface declares one type per
/// profile and implements, on it, the marker interface of every operation the profile's
/// cut holds, so a call the profile may not make does not compile.
/// </summary>
public interface IProfile
{
    /// <summary>The profile's name (<c>acme-ci</c>).</summary>
    static abstract string Name { get; }

    /// <summary>
    /// What its environment variables carry (<c>ACME_CI</c> in <c>INORBIT_ACME_CI_TOKEN</c>);
    /// empty for <see cref="PublicProfile"/>, which reads the bare <c>INORBIT_*</c> names.
    /// </summary>
    static abstract string Env { get; }
}

/// <summary>
/// The runtime's own profile: what any credential with the public scopes may call. Its
/// operations are the public surface in <c>InOrbit.Sdk.Api</c>.
/// </summary>
public sealed partial class PublicProfile : IProfile
{
    private PublicProfile()
    {
    }

    /// <inheritdoc/>
    public static string Name => "public";

    /// <inheritdoc/>
    public static string Env => string.Empty;
}
