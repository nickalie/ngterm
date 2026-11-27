package com.nickalie.ngterm.android.ui.machines

import androidx.annotation.StringRes
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.ConnectTimeoutRange
import com.nickalie.ngterm.android.data.SSH_PORT
import com.nickalie.ngterm.android.data.StoredMachine

/** A field of the machine form. */
enum class MachineField { Label, Host, Port, User, Key, Timeout }

/** What is wrong with a field, in the words the form shows under it. */
enum class FieldProblem(
    val field: MachineField,
    @param:StringRes val message: Int,
) {
    HostMissing(MachineField.Host, R.string.form_host_missing),
    HostHasSpace(MachineField.Host, R.string.form_host_space),
    HostHasUser(MachineField.Host, R.string.form_host_user),
    HostHasPort(MachineField.Host, R.string.form_host_port),
    PortInvalid(MachineField.Port, R.string.form_port_invalid),
    UserMissing(MachineField.User, R.string.form_user_missing),
    UserHasSpace(MachineField.User, R.string.form_user_space),
    KeyMissing(MachineField.Key, R.string.form_key_missing),
    TimeoutInvalid(MachineField.Timeout, R.string.form_timeout_invalid),
}

private const val MAX_PORT = 65535

// `host:22`, but not an IPv6 address, which has several colons
private val HOST_WITH_PORT = Regex("^[^:]+:[0-9]*$")

/**
 * The machine form's fields as typed: the port and the connect timeout as text, so a half-typed one can be shown and
 * corrected; an empty [timeout] takes Settings'.
 */
data class MachineDraft(
    val label: String = "",
    val host: String = "",
    val port: String = SSH_PORT.toString(),
    val user: String = "",
    val keyId: String = "",
    val timeout: String = "",
) {
    /** What is wrong with it, at most one problem per field. */
    fun problems(): Map<MachineField, FieldProblem> {
        val host = host.trim()
        val user = user.trim()
        val port = port.trim().toIntOrNull()
        return listOfNotNull(
            when {
                host.isEmpty() -> FieldProblem.HostMissing
                host.any(Char::isWhitespace) -> FieldProblem.HostHasSpace
                '@' in host -> FieldProblem.HostHasUser
                HOST_WITH_PORT.matches(host) -> FieldProblem.HostHasPort
                else -> null
            },
            FieldProblem.PortInvalid.takeIf { port == null || port !in 1..MAX_PORT },
            when {
                user.isEmpty() -> FieldProblem.UserMissing
                user.any(Char::isWhitespace) -> FieldProblem.UserHasSpace
                else -> null
            },
            FieldProblem.KeyMissing.takeIf { keyId.isEmpty() },
            FieldProblem.TimeoutInvalid.takeIf { timeout.isNotBlank() && timeoutSeconds() !in ConnectTimeoutRange },
        ).associateBy { it.field }
    }

    /** The machine it describes, as [id]; only for a draft with no [problems]. */
    fun toMachine(id: String): StoredMachine {
        check(problems().isEmpty()) { "the draft has problems" }
        return StoredMachine(
            id = id,
            label = label.trim(),
            host = host.trim(),
            port = port.trim().toInt(),
            user = user.trim(),
            keyId = keyId,
            connectTimeout = timeoutSeconds(),
        )
    }

    private fun timeoutSeconds(): Int? = timeout.trim().toIntOrNull()

    companion object {
        /** The draft that edits [machine]. */
        fun of(machine: StoredMachine) =
            MachineDraft(
                machine.label,
                machine.host,
                machine.port.toString(),
                machine.user,
                machine.keyId,
                machine.connectTimeout?.toString().orEmpty(),
            )
    }
}
