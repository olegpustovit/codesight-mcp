interface User {
    name: string;
}

/** Get user by ID. */
function getUser(id: number): User {
    return { name: "Test" };
}

class UserService {
    private users: User[] = [];

    findById(id: number): User | undefined {
        return this.users.find(u => u.id === id);
    }
}

type ID = string | number;

export function UserList() {
  return (
    <ul>
      {UserService.getUsers().map((user) => (
          <li>{user.name}</li>
      ))}
    </ul>
  )
}
