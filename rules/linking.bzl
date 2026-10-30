"""Resolve automatic package linkage without overriding explicit configurations."""

def _impl(ctx):
    automatic = ctx.attrs.automatic[ConstraintValueInfo]
    static = ctx.attrs.static[ConstraintValueInfo]

    def apply(platform):
        constraints = platform.configuration.constraints
        selected = constraints.get(automatic.setting.label)
        if selected != None and selected.label != automatic.label:
            return platform
        updated = dict(constraints)
        updated[static.setting.label] = static
        return PlatformInfo(
            label = platform.label + "-coldeer-static",
            configuration = ConfigurationInfo(
                constraints = updated,
                values = platform.configuration.values,
            ),
        )

    return [DefaultInfo(), TransitionInfo(impl = apply)]

package_linking = rule(
    impl = _impl,
    is_configuration_rule = True,
    attrs = {
        "automatic": attrs.dep(default = "//constraints:link_style[auto]"),
        "static": attrs.dep(default = "//constraints:link_style[static_pic]"),
    },
)
