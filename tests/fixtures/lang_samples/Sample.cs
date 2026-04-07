using System;
using System.Collections.Generic;

namespace SampleApp
{
    public class UserService
    {
        public UserService() {}

        public string GetUser(int userId) { return "user-" + userId.ToString(); }

        public bool DeleteUser(int userId) { return true; }
    }

    public interface IRepository
    {
        List<string> GetAll();
    }

    public enum Status { Pending, Active, Done }
}
