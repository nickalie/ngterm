"""Owned session-bus discovery/status service for a Linux UI test instance.

AccessKit checks IsEnabled on its session bus before using AT_SPI_BUS_ADDRESS.
Gio implements Properties.Get/GetAll from these declarations. This helper exposes
only the desktop's standard accessibility discovery interface, never app controls.
"""
import os

XML = '''<node>
<interface name="org.a11y.Bus">
  <method name="GetAddress"><arg type="s" direction="out"/></method>
</interface>
<interface name="org.a11y.Status">
  <property name="IsEnabled" type="b" access="read"/>
  <property name="ScreenReaderEnabled" type="b" access="read"/>
</interface>
</node>'''


def serve() -> None:
    from gi.repository import Gio, GLib
    connection = Gio.DBusConnection.new_for_address_sync(
        os.environ['DBUS_SESSION_BUS_ADDRESS'],
        Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
        None, None)
    def method(conn, sender, path, interface, name, args, invocation):
        if interface == 'org.a11y.Bus' and name == 'GetAddress':
            invocation.return_value(GLib.Variant('(s)', (os.environ['AT_SPI_BUS_ADDRESS'],)))
        else:
            invocation.return_dbus_error('org.freedesktop.DBus.Error.UnknownMethod', name)
    def get_property(conn, sender, path, interface, name):
        return GLib.Variant('b', name == 'IsEnabled')
    info = Gio.DBusNodeInfo.new_for_xml(XML)
    for interface in info.interfaces:
        connection.register_object('/org/a11y/bus', interface, method, get_property, None)
    result = connection.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
                                  'org.freedesktop.DBus', 'RequestName',
                                  GLib.Variant('(su)', ('org.a11y.Bus', 4)),
                                  GLib.VariantType.new('(u)'), Gio.DBusCallFlags.NONE, 5000, None)
    if result.unpack() != (1,):
        raise RuntimeError('instance accessibility status name already owned')
    GLib.MainLoop().run()


if __name__ == '__main__':
    serve()
